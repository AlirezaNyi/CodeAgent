-- Migration 0004: optional execution DAG node status (Phase 3 Slice C)

CREATE TABLE IF NOT EXISTS task_dag_nodes (
    task_id TEXT NOT NULL REFERENCES tasks(id),
    node_id TEXT NOT NULL,
    role TEXT NOT NULL,
    objective TEXT NOT NULL,
    paths_json TEXT NOT NULL DEFAULT '[]',
    depends_on_json TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL,
    summary TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(task_id, node_id)
);

CREATE INDEX IF NOT EXISTS idx_task_dag_nodes_task ON task_dag_nodes(task_id);
