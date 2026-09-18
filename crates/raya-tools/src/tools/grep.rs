//! In-process ripgrep-style search.

use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use grep_matcher::Matcher;
use grep_regex::RegexMatcher;
use grep_searcher::{Searcher, sinks};
use ignore::WalkBuilder;
use raya_core::{ToolCallId, ToolResult, ToolSchema};
use serde_json::{Value, json};

use crate::Tool;
use crate::registry::{ToolContext, ToolError};

pub struct SearchGrepTool;

#[async_trait]
impl Tool for SearchGrepTool {
    fn name(&self) -> &str {
        "search.grep"
    }

    fn description(&self) -> &str {
        "Search repository text with ripgrep libraries (honors .gitignore)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string"},
                    "glob": {"type": "string"},
                    "max_matches": {"type": "integer"}
                },
                "required": ["pattern"]
            }),
        }
    }

    async fn execute(&self, ctx: &ToolContext, input: Value) -> Result<ToolResult, ToolError> {
        let pattern = input
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("pattern required".into()))?
            .to_string();
        let glob = input
            .get("glob")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let max_matches = input
            .get("max_matches")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as usize;

        let root = ctx.project_root.clone();
        let output = tokio::task::spawn_blocking(move || {
            search_repo(&root, &pattern, glob.as_deref(), max_matches)
        })
        .await
        .map_err(|e| ToolError::Message(e.to_string()))??;

        Ok(ToolResult::ok(ToolCallId::new(), self.name(), output))
    }
}

fn glob_match(pattern: &str, path: &str) -> bool {
    // Minimal glob: `*.ext` suffix, or substring if no `*`.
    if let Some(ext) = pattern.strip_prefix("*.") {
        return path.ends_with(ext) || path.ends_with(&format!(".{ext}"));
    }
    if pattern.contains('*') {
        let parts: Vec<&str> = pattern.split('*').collect();
        if parts.is_empty() {
            return true;
        }
        let mut rest = path;
        if !parts[0].is_empty() {
            if let Some(i) = rest.find(parts[0]) {
                rest = &rest[i + parts[0].len()..];
            } else {
                return false;
            }
        }
        for p in &parts[1..] {
            if p.is_empty() {
                continue;
            }
            if let Some(i) = rest.find(p) {
                rest = &rest[i + p.len()..];
            } else {
                return false;
            }
        }
        return true;
    }
    path.contains(pattern)
}

fn search_repo(
    root: &Path,
    pattern: &str,
    glob: Option<&str>,
    max_matches: usize,
) -> Result<String, ToolError> {
    let matcher = RegexMatcher::new(pattern)
        .map_err(|e| ToolError::InvalidInput(format!("invalid regex: {e}")))?;

    let mut builder = WalkBuilder::new(root);
    builder.hidden(false).git_ignore(true).git_global(true);

    let matches: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut searcher = Searcher::new();

    for entry in builder.build().filter_map(|e| e.ok()) {
        {
            let guard = matches.lock().unwrap_or_else(|e| e.into_inner());
            if guard.len() >= max_matches {
                break;
            }
        }
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path().to_path_buf();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        if let Some(g) = glob
            && !glob_match(g, &rel)
        {
            continue;
        }

        let matches2 = matches.clone();
        let matcher2 = matcher.clone();

        let _ = searcher.search_path(
            &matcher2,
            &path,
            sinks::UTF8(|line_num, line| {
                let mut guard = matches2.lock().unwrap_or_else(|e| e.into_inner());
                if guard.len() >= max_matches {
                    return Ok(false);
                }
                if matcher2.find(line.as_bytes()).ok().flatten().is_some() {
                    guard.push(format!("{rel}:{line_num}:{}", line.trim_end()));
                }
                Ok(guard.len() < max_matches)
            }),
        );
    }

    let guard = matches.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_empty() {
        Ok("no matches".into())
    } else {
        Ok(guard.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raya_core::TaskId;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn finds_text() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn hello_raya() {}\n").unwrap();
        let ctx = ToolContext::new(
            TaskId::new(),
            dir.path().to_path_buf(),
            CancellationToken::new(),
        );
        let r = SearchGrepTool
            .execute(&ctx, json!({"pattern": "hello_raya"}))
            .await
            .unwrap();
        assert!(r.output.contains("hello_raya"));
    }
}
