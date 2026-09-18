//! E2E: durable approval resume across orchestrator instances.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use raya_agent::Orchestrator;
use raya_core::{
    AgentTask, CompletionResponse, Config, PolicyAction, TaskPhase, TokenUsage, ToolCallId,
};
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

fn setup_project() -> (tempfile::TempDir, Arc<Store>, Config, raya_core::ProjectId) {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".raya")).unwrap();
    std::fs::write(
        dir.path().join(".raya/config.toml"),
        r#"
[llm]
provider = "mock"

[policy]
shell = "approval"
write = "auto"
read = "auto"
"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("README.md"), "# demo\n").unwrap();
    let store = Arc::new(Store::open(dir.path().join(".raya/raya.db")).unwrap());
    let project = store.get_or_create_project(dir.path(), "demo").unwrap();
    let mut config = Config::load(dir.path()).unwrap();
    config.policy.shell = PolicyAction::Approval;
    config.llm.provider = "mock".into();
    (dir, store, config, project.id)
}

#[tokio::test]
async fn approve_resumes_across_processes() {
    let (dir, store, config, project_id) = setup_project();
    let tools = Arc::new(default_registry(PolicyEngine::new(config.policy.clone())));

    let call_id = "00000000-0000-4000-8000-0000000000aa";
    let llm1 = Arc::new(MockProvider::new(vec![
        resp(json!({
            "type": "plan",
            "plan": {
                "steps": [{"id":"1","description":"echo","expected_tools":["shell.exec"]}],
                "verification": "none",
                "summary": "echo"
            }
        })),
        resp(json!({
            "type": "tool_call",
            "call": {
                "id": call_id,
                "name": "shell.exec",
                "input": {"command": "echo hi"}
            }
        })),
    ]));

    let task = AgentTask::new(project_id, "echo hi", 12, 40, 100_000, 40_000, None);
    let id = task.id;
    store.create_task(&task).unwrap();

    let orch1 = Orchestrator::with_router(
        store.clone(),
        tools.clone(),
        Arc::new(ModelRouter::single(llm1)),
        config.clone(),
        dir.path().to_path_buf(),
    );
    let paused = orch1
        .run(id, CancellationToken::new())
        .await
        .expect("pause");
    assert_eq!(paused.phase, TaskPhase::WaitingApproval);
    assert!(store.load_checkpoint(id).unwrap().is_some());

    let cid: ToolCallId = call_id.parse().unwrap();
    store.set_approval(id, cid, true).unwrap();

    let llm2 = Arc::new(MockProvider::new(vec![resp(
        json!({"type": "finish", "summary": "echoed"}),
    )]));
    let orch2 = Orchestrator::with_router(
        store.clone(),
        tools,
        Arc::new(ModelRouter::single(llm2)),
        config,
        dir.path().to_path_buf(),
    );
    let finished = orch2
        .run(id, CancellationToken::new())
        .await
        .expect("resume");
    assert_eq!(finished.phase, TaskPhase::Completed);
    assert!(store.load_checkpoint(id).unwrap().is_none());

    let events = store.list_events(id, None, 200).unwrap();
    let kinds: Vec<_> = events.iter().map(|(_, e)| e.kind.as_str()).collect();
    assert!(kinds.contains(&"approval.granted"));
    assert!(kinds.contains(&"tool.completed"));
    assert!(kinds.contains(&"task.completed"));
}

#[tokio::test]
async fn deny_resumes_and_completes() {
    let (dir, store, config, project_id) = setup_project();
    let tools = Arc::new(default_registry(PolicyEngine::new(config.policy.clone())));

    let call_id = "00000000-0000-4000-8000-0000000000bb";
    let llm1 = Arc::new(MockProvider::new(vec![
        resp(json!({
            "type": "plan",
            "plan": {
                "steps": [{"id":"1","description":"echo","expected_tools":["shell.exec"]}],
                "verification": "none",
                "summary": "echo"
            }
        })),
        resp(json!({
            "type": "tool_call",
            "call": {
                "id": call_id,
                "name": "shell.exec",
                "input": {"command": "echo hi"}
            }
        })),
    ]));

    let task = AgentTask::new(project_id, "echo hi", 12, 40, 100_000, 40_000, None);
    let id = task.id;
    store.create_task(&task).unwrap();

    let orch1 = Orchestrator::with_router(
        store.clone(),
        tools.clone(),
        Arc::new(ModelRouter::single(llm1)),
        config.clone(),
        dir.path().to_path_buf(),
    );
    let paused = orch1
        .run(id, CancellationToken::new())
        .await
        .expect("pause");
    assert_eq!(paused.phase, TaskPhase::WaitingApproval);

    let cid: ToolCallId = call_id.parse().unwrap();
    store.set_approval(id, cid, false).unwrap();

    let llm2 = Arc::new(MockProvider::new(vec![resp(
        json!({"type": "finish", "summary": "skipped shell"}),
    )]));
    let orch2 = Orchestrator::with_router(
        store.clone(),
        tools,
        Arc::new(ModelRouter::single(llm2)),
        config,
        dir.path().to_path_buf(),
    );
    let finished = orch2
        .run(id, CancellationToken::new())
        .await
        .expect("resume deny");
    assert_eq!(finished.phase, TaskPhase::Completed);

    let events = store.list_events(id, None, 200).unwrap();
    let kinds: Vec<_> = events.iter().map(|(_, e)| e.kind.as_str()).collect();
    assert!(kinds.contains(&"approval.denied"));
    assert!(kinds.contains(&"task.completed"));
}
