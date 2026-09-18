//! E2E: optional execution DAG over subagents (Phase 3 Slice C).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use raya_agent::Orchestrator;
use raya_core::{
    AgentTask, CompletionResponse, Config, EventKind, PolicyAction, TaskPhase, TokenUsage,
};
use raya_llm::{MockProvider, ModelRole, ModelRouter};
use raya_policy::PolicyEngine;
use raya_store::Store;
use raya_tools::default_registry;
use serde_json::json;
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

fn usage() -> TokenUsage {
    TokenUsage {
        prompt_tokens: 10,
        completion_tokens: 10,
        total_tokens: 20,
    }
}

fn resp(content: serde_json::Value) -> CompletionResponse {
    CompletionResponse {
        content: content.to_string(),
        structured: None,
        usage: usage(),
        model: Some("mock".into()),
        finish_reason: Some("stop".into()),
    }
}

fn base_config(dir: &std::path::Path) -> Config {
    std::fs::create_dir_all(dir.join(".raya")).unwrap();
    std::fs::write(
        dir.join(".raya/config.toml"),
        r#"
[llm]
provider = "mock"

[policy]
shell = "deny"
write = "auto"
read = "auto"

[subagent]
max_parallel = 3
max_dag_nodes = 8
"#,
    )
    .unwrap();
    std::fs::write(dir.join("README.md"), "# demo\n").unwrap();
    let mut config = Config::load(dir).unwrap();
    config.policy.shell = PolicyAction::Deny;
    config.llm.provider = "mock".into();
    config
}

#[tokio::test]
async fn sequential_dag_writes_two_files() {
    let dir = tempdir().unwrap();
    let config = base_config(dir.path());
    let store = Arc::new(Store::open(dir.path().join(".raya/raya.db")).unwrap());
    let project = store.get_or_create_project(dir.path(), "demo").unwrap();
    let tools = Arc::new(default_registry(PolicyEngine::new(config.policy.clone())));

    // Shared mock: plan → coder A write+finish → coder B write+finish → parent finish
    let llm = Arc::new(MockProvider::new(vec![
        resp(json!({
            "type": "plan",
            "plan": {
                "steps": [{"id":"1","description":"dag","expected_tools":[]}],
                "verification": "none",
                "summary": "two files via dag",
                "nodes": [
                    {
                        "id": "a",
                        "role": "coder",
                        "objective": "Write file a.txt with content alpha",
                        "paths": [],
                        "depends_on": []
                    },
                    {
                        "id": "b",
                        "role": "coder",
                        "objective": "Write file b.txt with content beta",
                        "paths": [],
                        "depends_on": ["a"]
                    }
                ]
            }
        })),
        resp(json!({
            "type": "tool_call",
            "call": {
                "id": "00000000-0000-4000-8000-0000000000a1",
                "name": "filesystem.write",
                "input": {"path": "a.txt", "content": "alpha\n"}
            }
        })),
        resp(json!({"type": "finish", "summary": "wrote a.txt"})),
        resp(json!({
            "type": "tool_call",
            "call": {
                "id": "00000000-0000-4000-8000-0000000000b1",
                "name": "filesystem.write",
                "input": {"path": "b.txt", "content": "beta\n"}
            }
        })),
        resp(json!({"type": "finish", "summary": "wrote b.txt"})),
        resp(json!({"type": "finish", "summary": "dag done"})),
    ]));
    let router = Arc::new(ModelRouter::single(llm));

    let task = AgentTask::new(project.id, "Write a and b", 20, 40, 100_000, 40_000, None);
    let id = task.id;
    store.create_task(&task).unwrap();

    let orch = Orchestrator::with_router(
        store.clone(),
        tools,
        router,
        config,
        dir.path().to_path_buf(),
    );
    let finished = orch
        .run(id, CancellationToken::new())
        .await
        .expect("orchestrator");

    assert_eq!(finished.phase, TaskPhase::Completed);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt"))
            .unwrap()
            .trim(),
        "alpha"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("b.txt"))
            .unwrap()
            .trim(),
        "beta"
    );

    let events = store.list_events(id, None, 300).unwrap();
    let dag_spawns: Vec<_> = events
        .iter()
        .filter(|(_, e)| e.kind == EventKind::AgentSpawned)
        .filter(|(_, e)| e.payload.get("dag_node_id").is_some())
        .collect();
    assert_eq!(dag_spawns.len(), 2);
    let ids: Vec<_> = dag_spawns
        .iter()
        .filter_map(|(_, e)| e.payload.get("dag_node_id").and_then(|v| v.as_str()))
        .collect();
    assert!(ids.contains(&"a"));
    assert!(ids.contains(&"b"));

    // DAG rows cleaned on Completed
    assert!(store.list_dag_nodes(id).unwrap().is_empty());
}

#[tokio::test]
async fn parallel_dag_two_roles() {
    let dir = tempdir().unwrap();
    let config = base_config(dir.path());
    let store = Arc::new(Store::open(dir.path().join(".raya/raya.db")).unwrap());
    let project = store.get_or_create_project(dir.path(), "demo").unwrap();
    let tools = Arc::new(default_registry(PolicyEngine::new(config.policy.clone())));

    let coding = Arc::new(MockProvider::new(vec![
        resp(json!({
            "type": "plan",
            "plan": {
                "steps": [{"id":"1","description":"parallel","expected_tools":[]}],
                "verification": "none",
                "summary": "parallel nodes",
                "nodes": [
                    {
                        "id": "write",
                        "role": "coder",
                        "objective": "Write parallel.txt",
                        "paths": [],
                        "depends_on": []
                    },
                    {
                        "id": "review",
                        "role": "reviewer",
                        "objective": "Approve the approach",
                        "paths": ["README.md"],
                        "depends_on": []
                    }
                ]
            }
        })),
        // coder subagent (same Coding lane as parent)
        resp(json!({
            "type": "tool_call",
            "call": {
                "id": "00000000-0000-4000-8000-0000000000c1",
                "name": "filesystem.write",
                "input": {"path": "parallel.txt", "content": "ok\n"}
            }
        })),
        resp(json!({"type": "finish", "summary": "wrote parallel.txt"})),
        // parent after DAG
        resp(json!({"type": "finish", "summary": "parallel dag done"})),
    ]));
    let review = Arc::new(MockProvider::new(vec![resp(
        json!({"type": "finish", "summary": "approve"}),
    )]));

    // Parent + coder share Coding; reviewer has its own queue for true parallel spawn.
    let router = Arc::new(ModelRouter::with_role_providers(
        coding,
        [(ModelRole::Review, review as Arc<_>)],
    ));

    let task = AgentTask::new(project.id, "Parallel dag", 20, 40, 100_000, 40_000, None);
    let id = task.id;
    store.create_task(&task).unwrap();

    let orch = Orchestrator::with_router(
        store.clone(),
        tools,
        router,
        config,
        dir.path().to_path_buf(),
    );
    let finished = orch
        .run(id, CancellationToken::new())
        .await
        .expect("orchestrator");

    assert_eq!(finished.phase, TaskPhase::Completed);
    assert!(
        std::fs::read_to_string(dir.path().join("parallel.txt"))
            .unwrap()
            .contains("ok")
    );
    let events = store.list_events(id, None, 300).unwrap();
    let dag_ids: Vec<_> = events
        .iter()
        .filter(|(_, e)| e.kind == EventKind::AgentSpawned)
        .filter_map(|(_, e)| {
            e.payload
                .get("dag_node_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();
    assert!(dag_ids.contains(&"write".to_string()));
    assert!(dag_ids.contains(&"review".to_string()));
}

#[tokio::test]
async fn dag_crash_resume_continues_remaining_nodes() {
    use raya_core::{
        DagNode, DagNodeStatus, ExecutionPlan, Message, PlanStep, TaskCheckpoint,
        VerificationStrategy,
    };

    let dir = tempdir().unwrap();
    let config = base_config(dir.path());
    let store = Arc::new(Store::open(dir.path().join(".raya/raya.db")).unwrap());
    let project = store.get_or_create_project(dir.path(), "demo").unwrap();
    let tools = Arc::new(default_registry(PolicyEngine::new(config.policy.clone())));

    let task = AgentTask::new(
        project.id,
        "Write a and b via dag",
        20,
        40,
        100_000,
        40_000,
        None,
    );
    let id = task.id;
    store.create_task(&task).unwrap();

    // Advance to Executing as if the parent had planned and started the DAG.
    store
        .transition_task(id, TaskPhase::Created, TaskPhase::Planning)
        .unwrap();
    store
        .transition_task(id, TaskPhase::Planning, TaskPhase::ContextBuilding)
        .unwrap();
    store
        .transition_task(id, TaskPhase::ContextBuilding, TaskPhase::Executing)
        .unwrap();

    let plan = ExecutionPlan {
        steps: vec![PlanStep {
            id: "1".into(),
            description: "dag".into(),
            expected_tools: vec![],
        }],
        verification: VerificationStrategy::None,
        summary: Some("two files via dag".into()),
        nodes: vec![
            DagNode {
                id: "a".into(),
                role: "coder".into(),
                objective: "Write file a.txt with content alpha".into(),
                paths: vec![],
                depends_on: vec![],
            },
            DagNode {
                id: "b".into(),
                role: "coder".into(),
                objective: "Write file b.txt with content beta".into(),
                paths: vec![],
                depends_on: vec!["a".into()],
            },
        ],
    };
    let mut task = store.get_task(id).unwrap().unwrap();
    task.plan = Some(plan.clone());
    store.update_task(&task).unwrap();

    store.replace_dag(id, &plan.nodes).unwrap();
    assert!(
        store
            .set_dag_node_status(id, "a", DagNodeStatus::Completed, Some("wrote a.txt"))
            .unwrap()
    );
    assert!(
        store
            .set_dag_node_status(id, "b", DagNodeStatus::Running, None)
            .unwrap()
    );
    // Node a already "completed" before the crash.
    std::fs::write(dir.path().join("a.txt"), "alpha\n").unwrap();

    let cp = TaskCheckpoint::new(
        id,
        vec![
            Message::system("system"),
            Message::user("Write a and b via dag"),
            Message::assistant(
                r#"{"type":"plan","plan":{"steps":[{"id":"1","description":"dag","expected_tools":[]}],"verification":"none","summary":"two files via dag","nodes":[{"id":"a","role":"coder","objective":"Write file a.txt with content alpha","paths":[],"depends_on":[]},{"id":"b","role":"coder","objective":"Write file b.txt with content beta","paths":[],"depends_on":["a"]}]}}"#,
            ),
        ],
        None,
    );
    store.save_checkpoint(&cp).unwrap();

    // Second run: only remaining node b + parent finish.
    let llm = Arc::new(MockProvider::new(vec![
        resp(json!({
            "type": "tool_call",
            "call": {
                "id": "00000000-0000-4000-8000-0000000000b1",
                "name": "filesystem.write",
                "input": {"path": "b.txt", "content": "beta\n"}
            }
        })),
        resp(json!({"type": "finish", "summary": "wrote b.txt"})),
        resp(json!({"type": "finish", "summary": "dag resumed done"})),
    ]));
    let router = Arc::new(ModelRouter::single(llm));

    let orch = Orchestrator::with_router(
        store.clone(),
        tools,
        router,
        config,
        dir.path().to_path_buf(),
    );
    let finished = orch
        .run(id, CancellationToken::new())
        .await
        .expect("orchestrator resume");

    assert_eq!(finished.phase, TaskPhase::Completed);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("b.txt"))
            .unwrap()
            .trim(),
        "beta"
    );
    // Terminal cleanup removes DAG rows (running cleared + deleted).
    assert!(store.list_dag_nodes(id).unwrap().is_empty());

    let events = store.list_events(id, None, 300).unwrap();
    let resumed = events.iter().any(|(_, e)| {
        e.kind == EventKind::PhaseChanged
            && e.payload.get("resumed") == Some(&json!(true))
            && e.payload.get("dag") == Some(&json!(true))
    });
    assert!(resumed, "expected dag resume PhaseChanged event");

    let dag_spawns: Vec<_> = events
        .iter()
        .filter(|(_, e)| e.kind == EventKind::AgentSpawned)
        .filter_map(|(_, e)| {
            e.payload
                .get("dag_node_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();
    assert_eq!(dag_spawns, vec!["b".to_string()]);
}
