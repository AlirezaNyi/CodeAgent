//! E2E: task memory is recorded and recalled on a later task.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use raya_agent::Orchestrator;
use raya_core::{AgentTask, Config, MemoryKind, PolicyAction, TaskPhase};
use raya_llm::{MockProvider, ModelRouter};
use raya_policy::PolicyEngine;
use raya_store::Store;
use raya_tools::default_registry;
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn task_memory_written_and_recalled() {
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

[memory]
enabled = true
max_items = 5
max_tokens = 2000
"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("README.md"), "# demo\n").unwrap();

    let store = Arc::new(Store::open(dir.path().join(".raya/raya.db")).unwrap());
    let project = store.get_or_create_project(dir.path(), "demo").unwrap();
    let mut config = Config::load(dir.path()).unwrap();
    config.policy.shell = PolicyAction::Deny;
    config.memory.enabled = true;

    let tools = Arc::new(default_registry(PolicyEngine::new(config.policy.clone())));

    // Task 1: default script writes hello.txt
    let task1 = AgentTask::new(
        project.id,
        "Write a hello.txt file",
        12,
        40,
        100_000,
        40_000,
        None,
    );
    let id1 = task1.id;
    store.create_task(&task1).unwrap();
    let orch1 = Orchestrator::with_router(
        store.clone(),
        tools.clone(),
        Arc::new(ModelRouter::single(
            Arc::new(MockProvider::default_script()),
        )),
        config.clone(),
        dir.path().to_path_buf(),
    );
    let finished1 = orch1
        .run(id1, CancellationToken::new())
        .await
        .expect("task1");
    assert_eq!(finished1.phase, TaskPhase::Completed);

    let memories = store
        .list_memories(project.id, Some(MemoryKind::Task), 10)
        .unwrap();
    assert!(
        !memories.is_empty(),
        "expected task memory after completion"
    );

    // Task 2: another write — context.retrieved should include memories
    let task2 = AgentTask::new(
        project.id,
        "Write another hello related note",
        12,
        40,
        100_000,
        40_000,
        None,
    );
    let id2 = task2.id;
    store.create_task(&task2).unwrap();
    let orch2 = Orchestrator::with_router(
        store.clone(),
        tools,
        Arc::new(ModelRouter::single(
            Arc::new(MockProvider::default_script()),
        )),
        config,
        dir.path().to_path_buf(),
    );
    let finished2 = orch2
        .run(id2, CancellationToken::new())
        .await
        .expect("task2");
    assert_eq!(finished2.phase, TaskPhase::Completed);

    let events = store.list_events(id2, None, 100).unwrap();
    let ctx = events
        .iter()
        .find(|(_, e)| e.kind.as_str() == "context.retrieved")
        .expect("context.retrieved");
    let mems = ctx
        .1
        .payload
        .get("memories")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        !mems.is_empty(),
        "expected memories in context.retrieved payload"
    );
}
