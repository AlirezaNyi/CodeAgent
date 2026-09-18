//! Filesystem read / write / patch tools.

use async_trait::async_trait;
use raya_core::{ToolCallId, ToolResult, ToolSchema};
use serde_json::{Value, json};

use crate::Tool;
use crate::path::safe_path;
use crate::registry::{ToolContext, ToolError};

pub struct FilesystemReadTool;

#[async_trait]
impl Tool for FilesystemReadTool {
    fn name(&self) -> &str {
        "filesystem.read"
    }

    fn description(&self) -> &str {
        "Read a file relative to the project root"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "max_bytes": {"type": "integer"}
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(&self, ctx: &ToolContext, input: Value) -> Result<ToolResult, ToolError> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("path required".into()))?;
        let max_bytes = input
            .get("max_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(ctx.max_output_bytes as u64) as usize;

        let full = safe_path(&ctx.project_root, path)?;
        let data = tokio::fs::read(&full).await?;
        let truncated = data.len() > max_bytes;
        let slice = if truncated { &data[..max_bytes] } else { &data };
        let output = String::from_utf8_lossy(slice).into_owned();

        Ok(ToolResult {
            call_id: ToolCallId::new(),
            name: self.name().into(),
            success: true,
            output,
            truncated,
            error: None,
            duration_ms: 0,
        })
    }
}

pub struct FilesystemWriteTool;

#[async_trait]
impl Tool for FilesystemWriteTool {
    fn name(&self) -> &str {
        "filesystem.write"
    }

    fn description(&self) -> &str {
        "Write content to a file relative to the project root"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn execute(&self, ctx: &ToolContext, input: Value) -> Result<ToolResult, ToolError> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("path required".into()))?;
        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("content required".into()))?;

        let full = safe_path(&ctx.project_root, path)?;
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&full, content).await?;

        Ok(ToolResult::ok(
            ToolCallId::new(),
            self.name(),
            format!("wrote {} bytes to {path}", content.len()),
        ))
    }
}

pub struct FilesystemPatchTool;

#[async_trait]
impl Tool for FilesystemPatchTool {
    fn name(&self) -> &str {
        "filesystem.patch"
    }

    fn description(&self) -> &str {
        "Apply search/replace patches to a file"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "replacements": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "search": {"type": "string"},
                                "replace": {"type": "string"}
                            },
                            "required": ["search", "replace"]
                        }
                    }
                },
                "required": ["path", "replacements"]
            }),
        }
    }

    async fn execute(&self, ctx: &ToolContext, input: Value) -> Result<ToolResult, ToolError> {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("path required".into()))?;
        let replacements = input
            .get("replacements")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ToolError::InvalidInput("replacements required".into()))?;

        let full = safe_path(&ctx.project_root, path)?;
        let mut content = tokio::fs::read_to_string(&full).await?;
        let mut applied = 0usize;

        for rep in replacements {
            let search = rep
                .get("search")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidInput("search required".into()))?;
            let replace = rep
                .get("replace")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidInput("replace required".into()))?;
            if content.contains(search) {
                content = content.replacen(search, replace, 1);
                applied += 1;
            }
        }

        tokio::fs::write(&full, &content).await?;
        Ok(ToolResult::ok(
            ToolCallId::new(),
            self.name(),
            format!("patched {path}: {applied} replacement(s)"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raya_core::TaskId;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    fn ctx(root: &std::path::Path) -> ToolContext {
        ToolContext::new(TaskId::new(), root.to_path_buf(), CancellationToken::new())
    }

    #[tokio::test]
    async fn read_write_patch() {
        let dir = tempdir().unwrap();
        let c = ctx(dir.path());
        FilesystemWriteTool
            .execute(&c, json!({"path": "a.txt", "content": "hello world"}))
            .await
            .unwrap();
        let r = FilesystemReadTool
            .execute(&c, json!({"path": "a.txt"}))
            .await
            .unwrap();
        assert_eq!(r.output, "hello world");
        FilesystemPatchTool
            .execute(
                &c,
                json!({
                    "path": "a.txt",
                    "replacements": [{"search": "world", "replace": "raya"}]
                }),
            )
            .await
            .unwrap();
        let r = FilesystemReadTool
            .execute(&c, json!({"path": "a.txt"}))
            .await
            .unwrap();
        assert_eq!(r.output, "hello raya");
    }
}
