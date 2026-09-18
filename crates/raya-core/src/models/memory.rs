//! Selective project/task memory records (RFC §15).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ids::{ProjectId, TaskId};

/// Memory category stored in SQLite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Task,
    Project,
    Decision,
    Agent,
}

impl MemoryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Project => "project",
            Self::Decision => "decision",
            Self::Agent => "agent",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "task" => Self::Task,
            "project" => Self::Project,
            "decision" => Self::Decision,
            "agent" => Self::Agent,
            _ => return None,
        })
    }
}

/// One selective memory entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryRecord {
    pub id: String,
    pub project_id: ProjectId,
    pub task_id: Option<TaskId>,
    pub kind: MemoryKind,
    pub key: Option<String>,
    pub content: String,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl MemoryRecord {
    pub fn new(
        project_id: ProjectId,
        kind: MemoryKind,
        content: impl Into<String>,
        task_id: Option<TaskId>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            project_id,
            task_id,
            kind,
            key: None,
            content: content.into(),
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kinds() {
        assert_eq!(MemoryKind::parse("Task"), Some(MemoryKind::Task));
        assert_eq!(MemoryKind::parse("decision"), Some(MemoryKind::Decision));
        assert!(MemoryKind::parse("unknown").is_none());
        assert_eq!(MemoryKind::Project.as_str(), "project");
    }
}
