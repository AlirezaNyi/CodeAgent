//! Context assembly with token budgets.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use raya_core::{HeuristicCounter, TokenCounter};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::debug;

use crate::rank::{RankExplanation, RankSignals, rank};
use crate::search::{extract_keywords, search_candidates};

#[derive(Debug, Error)]
pub enum ContextError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextSnippet {
    pub path: PathBuf,
    pub content: String,
    pub tokens: u64,
    pub explanation: RankExplanation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextBundle {
    pub snippets: Vec<ContextSnippet>,
    pub total_tokens: u64,
    pub dropped: Vec<PathBuf>,
    pub keywords: Vec<String>,
}

pub struct ContextEngine {
    max_tokens: u64,
    max_files: usize,
    counter: HeuristicCounter,
}

impl ContextEngine {
    pub fn new(max_tokens: u64, max_files: u32) -> Self {
        Self {
            max_tokens,
            max_files: max_files as usize,
            counter: HeuristicCounter,
        }
    }

    pub fn build(&self, root: &Path, request: &str) -> Result<ContextBundle, ContextError> {
        self.build_with_signals(root, request, &RankSignals::default())
    }

    pub fn build_with_signals(
        &self,
        root: &Path,
        request: &str,
        signals: &RankSignals,
    ) -> Result<ContextBundle, ContextError> {
        let keywords = extract_keywords(request);
        let candidates = search_candidates(root, &keywords, self.max_files);
        let ranked = rank(candidates, &keywords, signals);

        let mut snippets = Vec::new();
        let mut dropped = Vec::new();
        let mut seen = HashSet::new();
        let mut total_tokens = 0u64;

        for cand in ranked {
            let key = cand.path.to_string_lossy().to_string();
            if !seen.insert(key) {
                continue;
            }
            if snippets.len() >= self.max_files {
                dropped.push(cand.path);
                continue;
            }

            let full = root.join(&cand.path);
            let content = match std::fs::read_to_string(&full) {
                Ok(c) => truncate_window(&c, 8_000),
                Err(_) => {
                    if cand.preview_lines.is_empty() {
                        dropped.push(cand.path);
                        continue;
                    }
                    cand.preview_lines.join("\n")
                }
            };

            let header = format!("// file: {}\n", cand.path.display());
            let block = format!("{header}{content}");
            let tokens = self.counter.count(&block);
            if total_tokens + tokens > self.max_tokens {
                dropped.push(cand.path);
                continue;
            }
            total_tokens += tokens;
            snippets.push(ContextSnippet {
                path: cand.path,
                content: block,
                tokens,
                explanation: cand.explanation,
            });
        }

        debug!(
            files = snippets.len(),
            total_tokens,
            dropped = dropped.len(),
            "context assembled"
        );

        Ok(ContextBundle {
            snippets,
            total_tokens,
            dropped,
            keywords,
        })
    }

    pub fn render_prompt(&self, bundle: &ContextBundle) -> String {
        bundle
            .snippets
            .iter()
            .map(|s| s.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

fn truncate_window(content: &str, max_chars: usize) -> String {
    if content.chars().count() <= max_chars {
        return content.to_string();
    }
    content.chars().take(max_chars).collect::<String>() + "\n...[truncated]"
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn respects_token_budget() {
        let dir = tempdir().unwrap();
        for i in 0..10 {
            let body = "alpha beta gamma ".repeat(200);
            std::fs::write(dir.path().join(format!("f{i}.rs")), format!("// {body}")).unwrap();
        }
        let engine = ContextEngine::new(100, 50);
        let bundle = engine.build(dir.path(), "alpha beta gamma").unwrap();
        assert!(bundle.total_tokens <= 100);
        assert!(!bundle.snippets.is_empty() || !bundle.dropped.is_empty());
    }

    #[test]
    fn dedups_and_is_deterministic() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("login.rs"), "fn login() {}").unwrap();
        std::fs::write(dir.path().join("auth.rs"), "fn auth_login() {}").unwrap();
        let engine = ContextEngine::new(40_000, 50);
        let a = engine.build(dir.path(), "login auth").unwrap();
        let b = engine.build(dir.path(), "login auth").unwrap();
        assert_eq!(a.snippets.len(), b.snippets.len());
        assert_eq!(
            a.snippets.iter().map(|s| &s.path).collect::<Vec<_>>(),
            b.snippets.iter().map(|s| &s.path).collect::<Vec<_>>()
        );
        let paths: HashSet<_> = a.snippets.iter().map(|s| s.path.clone()).collect();
        assert_eq!(paths.len(), a.snippets.len());
    }
}
