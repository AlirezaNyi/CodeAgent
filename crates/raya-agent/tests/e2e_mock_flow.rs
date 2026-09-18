//! End-to-end mock LLM flow: task → plan → context → tool → completion.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use raya_agent::Orchestrator;
use raya_core::{AgentTask, Config, PolicyAction, TaskPhase};
use raya_llm::MockProvider;
use raya_policy::PolicyEngine;
use raya_store::Store;
use raya_tools::default_registry;
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn mock_llm_completes_write_task() {
    let dir = tempdir().unwrap();
    // Minimal project markers
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
    config.llm.provider = "mock".into();

    let tools = Arc::new(default_registry(PolicyEngine::new(config.policy.clone())));
    let llm = Arc::new(MockProvider::default_script());

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

    let orch = Orchestrator::new(store.clone(), tools, llm, config, dir.path().to_path_buf());
    let finished = orch
        .run(id, CancellationToken::new())
        .await
        .expect("orchestrator");

    assert_eq!(finished.phase, TaskPhase::Completed);
    let hello = std::fs::read_to_string(dir.path().join("hello.txt")).unwrap();
    assert!(hello.contains("hello from raya"));

    let events = store.list_events(id, None, 100).unwrap();
    let kinds: Vec<_> = events.iter().map(|(_, e)| e.kind.as_str()).collect();
    assert!(kinds.contains(&"task.created"));
    assert!(kinds.contains(&"plan.created"));
    assert!(kinds.contains(&"context.retrieved"));
    assert!(kinds.contains(&"tool.completed"));
    assert!(kinds.contains(&"task.completed"));
}
