//! Optional execution DAG wave scheduling (Phase 3 Slice C).

use std::collections::HashMap;

use raya_core::{DagNode, DagNodeStatus};

/// Node ids that are `Pending` and whose deps are all `Completed`.
pub fn ready_nodes<'a>(
    nodes: &'a [DagNode],
    status: &HashMap<String, DagNodeStatus>,
) -> Vec<&'a DagNode> {
    nodes
        .iter()
        .filter(|n| {
            status.get(&n.id).copied() == Some(DagNodeStatus::Pending)
                && n.depends_on
                    .iter()
                    .all(|d| status.get(d).copied() == Some(DagNodeStatus::Completed))
        })
        .collect()
}

/// Mark pending nodes whose deps include Failed or Skipped as Skipped.
/// Returns newly skipped node ids.
pub fn skip_blocked(nodes: &[DagNode], status: &mut HashMap<String, DagNodeStatus>) -> Vec<String> {
    let mut newly = Vec::new();
    let mut changed = true;
    while changed {
        changed = false;
        for n in nodes {
            if status.get(&n.id).copied() != Some(DagNodeStatus::Pending) {
                continue;
            }
            let blocked = n.depends_on.iter().any(|d| {
                matches!(
                    status.get(d).copied(),
                    Some(DagNodeStatus::Failed | DagNodeStatus::Skipped)
                )
            });
            if blocked {
                status.insert(n.id.clone(), DagNodeStatus::Skipped);
                newly.push(n.id.clone());
                changed = true;
            }
        }
    }
    newly
}

pub fn all_terminal(status: &HashMap<String, DagNodeStatus>) -> bool {
    status.values().all(|s| s.is_terminal())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, deps: &[&str]) -> DagNode {
        DagNode {
            id: id.into(),
            role: "coder".into(),
            objective: id.into(),
            paths: vec![],
            depends_on: deps.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[test]
    fn ready_respects_deps() {
        let nodes = vec![node("a", &[]), node("b", &["a"])];
        let mut status = HashMap::from([
            ("a".into(), DagNodeStatus::Pending),
            ("b".into(), DagNodeStatus::Pending),
        ]);
        let ready = ready_nodes(&nodes, &status);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "a");
        status.insert("a".into(), DagNodeStatus::Completed);
        let ready = ready_nodes(&nodes, &status);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "b");
    }

    #[test]
    fn skip_on_failed_dep() {
        let nodes = vec![node("a", &[]), node("b", &["a"]), node("c", &["b"])];
        let mut status = HashMap::from([
            ("a".into(), DagNodeStatus::Failed),
            ("b".into(), DagNodeStatus::Pending),
            ("c".into(), DagNodeStatus::Pending),
        ]);
        let skipped = skip_blocked(&nodes, &mut status);
        assert!(skipped.contains(&"b".into()));
        assert!(skipped.contains(&"c".into()));
        assert_eq!(status["b"], DagNodeStatus::Skipped);
        assert_eq!(status["c"], DagNodeStatus::Skipped);
    }
}
