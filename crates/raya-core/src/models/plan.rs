//! Execution plan models.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ids::TaskId;

/// Structured execution plan produced by the planner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionPlan {
    pub steps: Vec<PlanStep>,
    pub verification: VerificationStrategy,
    pub summary: Option<String>,
    /// Optional DAG overlay (Phase 3 Slice C). Empty = sequential LLM loop only.
    #[serde(default)]
    pub nodes: Vec<DagNode>,
}

impl ExecutionPlan {
    pub fn new(steps: Vec<PlanStep>, verification: VerificationStrategy) -> Self {
        Self {
            steps,
            verification,
            summary: None,
            nodes: Vec::new(),
        }
    }
}

/// One ordered step in an execution plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanStep {
    pub id: String,
    pub description: String,
    pub expected_tools: Vec<String>,
}

/// How the agent should verify work after execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStrategy {
    None,
    #[default]
    Test,
    Build,
    TestAndBuild,
    Custom {
        command: String,
    },
}

/// Optional DAG node scheduled as a bounded subagent (RFC §13).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DagNode {
    pub id: String,
    /// `planner` | `coder` | `reviewer` | `debugger`
    pub role: String,
    pub objective: String,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

/// Persisted status of a DAG node for a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DagNodeStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

impl DagNodeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => Self::Pending,
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "skipped" => Self::Skipped,
            _ => return None,
        })
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Skipped)
    }
}

/// Row stored in `task_dag_nodes` (observability / CLI / HTTP).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskDagNodeRecord {
    pub task_id: TaskId,
    pub node_id: String,
    pub role: String,
    pub objective: String,
    pub paths: Vec<String>,
    pub depends_on: Vec<String>,
    pub status: DagNodeStatus,
    pub summary: Option<String>,
    pub updated_at: DateTime<Utc>,
}

fn known_role(role: &str) -> bool {
    matches!(
        role.trim().to_ascii_lowercase().as_str(),
        "planner" | "coder" | "reviewer" | "debugger"
    )
}

/// Validate optional DAG nodes. Empty is always valid.
pub fn validate_dag(nodes: &[DagNode], max_nodes: u32) -> Result<(), String> {
    if nodes.is_empty() {
        return Ok(());
    }
    if nodes.len() as u32 > max_nodes {
        return Err(format!(
            "dag has {} nodes; max_dag_nodes is {max_nodes}",
            nodes.len()
        ));
    }
    let mut ids = HashSet::new();
    for n in nodes {
        if n.id.trim().is_empty() {
            return Err("dag node id must be non-empty".into());
        }
        if !ids.insert(n.id.clone()) {
            return Err(format!("duplicate dag node id: {}", n.id));
        }
        if !known_role(&n.role) {
            return Err(format!(
                "unknown dag role \"{}\" on node {} (expected planner|coder|reviewer|debugger)",
                n.role, n.id
            ));
        }
    }
    for n in nodes {
        for dep in &n.depends_on {
            if !ids.contains(dep) {
                return Err(format!("node {} depends on missing node \"{dep}\"", n.id));
            }
        }
    }
    // Cycle detection via DFS (white/gray/black).
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for n in nodes {
        adj.entry(n.id.as_str()).or_default();
        for dep in &n.depends_on {
            // edge dep -> n (dep must complete before n)
            adj.entry(dep.as_str()).or_default().push(n.id.as_str());
        }
    }
    let mut color: HashMap<&str, u8> = HashMap::new();
    fn dfs<'a>(
        u: &'a str,
        adj: &HashMap<&'a str, Vec<&'a str>>,
        color: &mut HashMap<&'a str, u8>,
    ) -> Result<(), String> {
        color.insert(u, 1);
        if let Some(neis) = adj.get(u) {
            for &v in neis {
                match color.get(v).copied().unwrap_or(0) {
                    1 => return Err(format!("dag cycle involving node {v}")),
                    0 => dfs(v, adj, color)?,
                    _ => {}
                }
            }
        }
        color.insert(u, 2);
        Ok(())
    }
    for n in nodes {
        if color.get(n.id.as_str()).copied().unwrap_or(0) == 0 {
            dfs(n.id.as_str(), &adj, &mut color)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_nodes_default_on_legacy_plan() {
        let v = json!({
            "steps": [{"id":"1","description":"x","expected_tools":[]}],
            "verification": "none",
            "summary": "s"
        });
        let p: ExecutionPlan = serde_json::from_value(v).unwrap();
        assert!(p.nodes.is_empty());
    }

    #[test]
    fn validate_ok_chain() {
        let nodes = vec![
            DagNode {
                id: "a".into(),
                role: "coder".into(),
                objective: "A".into(),
                paths: vec![],
                depends_on: vec![],
            },
            DagNode {
                id: "b".into(),
                role: "coder".into(),
                objective: "B".into(),
                paths: vec![],
                depends_on: vec!["a".into()],
            },
        ];
        assert!(validate_dag(&nodes, 8).is_ok());
    }

    #[test]
    fn validate_rejects_cycle() {
        let nodes = vec![
            DagNode {
                id: "a".into(),
                role: "coder".into(),
                objective: "A".into(),
                paths: vec![],
                depends_on: vec!["b".into()],
            },
            DagNode {
                id: "b".into(),
                role: "coder".into(),
                objective: "B".into(),
                paths: vec![],
                depends_on: vec!["a".into()],
            },
        ];
        assert!(validate_dag(&nodes, 8).unwrap_err().contains("cycle"));
    }

    #[test]
    fn validate_rejects_missing_dep() {
        let nodes = vec![DagNode {
            id: "a".into(),
            role: "coder".into(),
            objective: "A".into(),
            paths: vec![],
            depends_on: vec!["missing".into()],
        }];
        assert!(validate_dag(&nodes, 8).unwrap_err().contains("missing"));
    }

    #[test]
    fn validate_rejects_unknown_role() {
        let nodes = vec![DagNode {
            id: "a".into(),
            role: "wizard".into(),
            objective: "A".into(),
            paths: vec![],
            depends_on: vec![],
        }];
        assert!(validate_dag(&nodes, 8).unwrap_err().contains("unknown"));
    }

    #[test]
    fn validate_rejects_too_many() {
        let nodes: Vec<_> = (0..3)
            .map(|i| DagNode {
                id: format!("n{i}"),
                role: "coder".into(),
                objective: "x".into(),
                paths: vec![],
                depends_on: vec![],
            })
            .collect();
        assert!(
            validate_dag(&nodes, 2)
                .unwrap_err()
                .contains("max_dag_nodes")
        );
    }

    #[test]
    fn dag_status_roundtrip() {
        assert_eq!(
            DagNodeStatus::parse("running"),
            Some(DagNodeStatus::Running)
        );
        assert_eq!(DagNodeStatus::Completed.as_str(), "completed");
    }
}
