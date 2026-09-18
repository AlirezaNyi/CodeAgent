//! MCP control-plane ops against a mock LLM project (no JSON-RPC client).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use raya_core::{AgentTask, TaskCheckpoint, TaskPhase, ToolCall, ToolCallId};
use raya_mcp::{RayaMcpContext, RayaMcpServer, tool_names};
use serde_json::json;
use tempfile::tempdir;

fn write_mock_project(dir: &std::path::Path) {
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
"#,
    )
    .unwrap();
    std::fs::write(dir.join("README.md"), "# demo\n").unwrap();
}

#[tokio::test]
async fn mcp_run_completes_hello_via_ops() {
    let dir = tempdir().unwrap();
    write_mock_project(dir.path());

    let ctx = Arc::new(RayaMcpContext::open(dir.path()).expect("open mcp ctx"));
    let result = ctx.run("Write a hello.txt file").await.expect("run");

    assert_eq!(result["phase"], "completed");
    let hello = std::fs::read_to_string(dir.path().join("hello.txt")).unwrap();
    assert!(hello.contains("hello from raya"));

    let task_id = result["id"].as_str().unwrap();
    let status = ctx.status(Some(task_id)).unwrap();
    assert_eq!(status["phase"], "completed");

    let logs = ctx.logs(task_id, Some(100)).unwrap();
    let kinds: Vec<&str> = logs["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["kind"].as_str())
        .collect();
    assert!(kinds.contains(&"task.completed"));
}

#[tokio::test]
async fn approve_rejects_wrong_phase_and_missing_checkpoint() {
    let dir = tempdir().unwrap();
    write_mock_project(dir.path());
    let ctx = Arc::new(RayaMcpContext::open(dir.path()).unwrap());

    let task = AgentTask::new(ctx.project_id, "noop", 4, 8, 10_000, 4_000, None);
    let tid = task.id.to_string();
    ctx.store.create_task(&task).unwrap();

    let fake_call = ToolCallId::new().to_string();
    let err = ctx
        .approve(&tid, &fake_call, true, true)
        .await
        .expect_err("must reject non-waiting task");
    assert!(err.to_string().contains("not waiting for approval"));

    ctx.store
        .transition_task(task.id, TaskPhase::Created, TaskPhase::Planning)
        .unwrap();
    ctx.store
        .transition_task(task.id, TaskPhase::Planning, TaskPhase::WaitingApproval)
        .unwrap();

    let err = ctx
        .approve(&tid, &fake_call, true, true)
        .await
        .expect_err("no checkpoint");
    assert!(err.to_string().contains("checkpoint"), "unexpected: {err}");

    let pending = ToolCall::new("shell.exec", json!({"command": "echo"}));
    let expected = pending.id;
    ctx.store
        .save_checkpoint(&TaskCheckpoint::new(task.id, vec![], Some(pending)))
        .unwrap();
    let wrong = ToolCallId::new().to_string();
    let err = ctx
        .approve(&tid, &wrong, true, true)
        .await
        .expect_err("call_id mismatch");
    assert!(
        err.to_string().contains("call_id") && err.to_string().contains(&format!("{expected:?}")),
        "unexpected: {err}"
    );
}

#[test]
fn tool_names_stable_and_server_constructs() {
    let names = tool_names();
    assert_eq!(names.len(), 11);
    assert!(names.contains(&"raya_run"));
    assert!(names.contains(&"raya_memory_forget"));

    let dir = tempdir().unwrap();
    write_mock_project(dir.path());
    let ctx = Arc::new(RayaMcpContext::open(dir.path()).unwrap());
    let _server = RayaMcpServer::new(ctx);
}
