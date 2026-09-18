//! E2E: review_on_finish spawns Reviewer subagent.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use raya_agent::Orchestrator;
use raya_core::{AgentTask, CompletionResponse, Config, PolicyAction, TaskPhase, TokenUsage};
use raya_llm::{MockProvider, ModelRouter};
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

#[tokio::test]
async fn review_on_finish_emits_agent_events() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".raya")).unwrap();
    std::fs::write(
        dir.path().join(".raya/config.toml"),
        r#"
[llm]
provider = "mock"

[policy]
shell = "deny"
write = "auto"
read = "auto"

[subagent]
review_on_finish = true
"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("README.md"), "# demo\n").unwrap();

    let store = Arc::new(Store::open(dir.path().join(".raya/raya.db")).unwrap());
    let project = store.get_or_create_project(dir.path(), "demo").unwrap();

    let mut config = Config::load(dir.path()).unwrap();
    config.policy.shell = PolicyAction::Deny;
    config.llm.provider = "mock".into();
    config.subagent.review_on_finish = true;

    let tools = Arc::new(default_registry(PolicyEngine::new(config.policy.clone())));
    // plan → write → finish → reviewer finish(approve)
    let llm = Arc::new(MockProvider::new(vec![
        resp(json!({
            "type": "plan",
            "plan": {
                "steps": [{
                    "id": "1",
                    "description": "Write hello file",
                    "expected_tools": ["filesystem.write"]
                }],
                "verification": "none",
                "summary": "Write hello"
            }
        })),
        resp(json!({
            "type": "tool_call",
            "call": {
                "id": "00000000-0000-4000-8000-000000000001",
                "name": "filesystem.write",
                "input": {"path": "hello.txt", "content": "hello from raya\n"}
            }
        })),
        resp(json!({"type": "finish", "summary": "Wrote hello.txt successfully"})),
        resp(json!({"type": "finish", "summary": "Looks good"})),
    ]));
    let router = Arc::new(ModelRouter::single(llm));

    let task = AgentTask::new(
        project.id,
        "Write a hello.txt file",
        12,
        40,
        100_000,
        40_000,
        None,
    );
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
    let events = store.list_events(id, None, 200).unwrap();
    let kinds: Vec<_> = events.iter().map(|(_, e)| e.kind.as_str()).collect();
    assert!(kinds.contains(&"agent.spawned"));
    assert!(kinds.contains(&"agent.completed"));
    assert!(kinds.contains(&"task.completed"));
}

#[tokio::test]
async fn delegate_planner_records_plan() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".raya")).unwrap();
    std::fs::write(
        dir.path().join(".raya/config.toml"),
        r#"
[llm]
provider = "mock"

[policy]
shell = "deny"
write = "auto"
read = "auto"
"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("README.md"), "# demo\n").unwrap();

    let store = Arc::new(Store::open(dir.path().join(".raya/raya.db")).unwrap());
    let project = store.get_or_create_project(dir.path(), "demo").unwrap();
    let mut config = Config::load(dir.path()).unwrap();
    config.policy.shell = PolicyAction::Deny;

    let tools = Arc::new(default_registry(PolicyEngine::new(config.policy.clone())));
    // main: delegate → (planner plan) → finish
    // Shared mock queue: main consumes first, planner second, main third
    let llm = Arc::new(MockProvider::new(vec![
        resp(json!({
            "type": "delegate",
            "role": "planner",
            "objective": "Plan writing hello.txt",
            "paths": ["README.md"]
        })),
        resp(json!({
            "type": "plan",
            "plan": {
                "steps": [{
                    "id": "1",
                    "description": "Write hello",
                    "expected_tools": ["filesystem.write"]
                }],
                "verification": "none",
                "summary": "from planner"
            }
        })),
        resp(json!({
            "type": "tool_call",
            "call": {
                "id": "00000000-0000-4000-8000-000000000002",
                "name": "filesystem.write",
                "input": {"path": "hello.txt", "content": "via delegate\n"}
            }
        })),
        resp(json!({"type": "finish", "summary": "done via delegate"})),
    ]));
    let router = Arc::new(ModelRouter::single(llm));

    let task = AgentTask::new(project.id, "Write hello", 12, 40, 100_000, 40_000, None);
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
    assert!(finished.plan.is_some());
    let events = store.list_events(id, None, 200).unwrap();
    let kinds: Vec<_> = events.iter().map(|(_, e)| e.kind.as_str()).collect();
    assert!(kinds.contains(&"agent.spawned"));
    assert!(kinds.contains(&"plan.created"));
    assert!(
        std::fs::read_to_string(dir.path().join("hello.txt"))
            .unwrap()
            .contains("via delegate")
    );
}
