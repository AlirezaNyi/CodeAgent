//! Relevance ranking with explanations.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RankExplanation {
    pub lexical_hits: usize,
    pub path_match: bool,
    pub symbol_match: bool,
    pub git_recent: bool,
    pub fts_match: bool,
    pub size_penalty: f64,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RankedCandidate {
    pub path: PathBuf,
    pub hit_count: usize,
    pub size_bytes: u64,
    pub preview_lines: Vec<String>,
    pub score: f64,
    pub explanation: RankExplanation,
    #[serde(default)]
    pub symbol_match: bool,
    #[serde(default)]
    pub git_recent: bool,
    #[serde(default)]
    pub fts_match: bool,
}

/// Extra signals applied during ranking.
#[derive(Debug, Clone, Default)]
pub struct RankSignals {
    pub symbol_paths: HashSet<String>,
    pub git_recent_paths: HashSet<String>,
    pub fts_paths: HashSet<String>,
}

pub fn rank(
    mut candidates: Vec<RankedCandidate>,
    keywords: &[String],
    signals: &RankSignals,
) -> Vec<RankedCandidate> {
    for c in &mut candidates {
        let path_l = c.path.to_string_lossy().to_ascii_lowercase();
        let path_key = c.path.to_string_lossy().replace('\\', "/");
        let path_match = keywords.iter().any(|k| path_l.contains(k));
        let symbol_match = c.symbol_match
            || signals
                .symbol_paths
                .iter()
                .any(|p| p == &path_key || path_key.ends_with(p));
        let git_recent = c.git_recent
            || signals
                .git_recent_paths
                .iter()
                .any(|p| p == &path_key || path_key.ends_with(p));
        let fts_match = c.fts_match
            || signals
                .fts_paths
                .iter()
                .any(|p| p == &path_key || path_key.ends_with(p));

        let lexical = c.hit_count as f64 * 10.0;
        let path_score = if path_match { 25.0 } else { 0.0 };
        let symbol_score = if symbol_match { 30.0 } else { 0.0 };
        let git_score = if git_recent { 15.0 } else { 0.0 };
        let fts_score = if fts_match { 20.0 } else { 0.0 };
        let size_penalty = (c.size_bytes as f64 / 10_000.0).min(20.0);
        let score = lexical + path_score + symbol_score + git_score + fts_score - size_penalty;

        let mut notes = Vec::new();
        if c.hit_count > 0 {
            notes.push(format!("{} lexical hits", c.hit_count));
        }
        if path_match {
            notes.push("path/name matched keywords".into());
        }
        if symbol_match {
            notes.push("symbol name matched keywords".into());
        }
        if git_recent {
            notes.push("recently touched in git".into());
        }
        if fts_match {
            notes.push("FTS index match".into());
        }
        if size_penalty > 0.0 {
            notes.push(format!("size penalty {size_penalty:.1}"));
        }

        c.symbol_match = symbol_match;
        c.git_recent = git_recent;
        c.fts_match = fts_match;
        c.score = score;
        c.explanation = RankExplanation {
            lexical_hits: c.hit_count,
            path_match,
            symbol_match,
            git_recent,
            fts_match,
            size_penalty,
            notes,
        };
    }

    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ranking_boosts_symbol_and_fts() {
        let candidates = vec![RankedCandidate {
            path: PathBuf::from("src/auth.rs"),
            hit_count: 1,
            size_bytes: 100,
            preview_lines: vec![],
            score: 0.0,
            explanation: Default::default(),
            symbol_match: false,
            git_recent: false,
            fts_match: false,
        }];
        let signals = RankSignals {
            symbol_paths: HashSet::from(["src/auth.rs".into()]),
            fts_paths: HashSet::from(["src/auth.rs".into()]),
            ..Default::default()
        };
        let ranked = rank(candidates, &["auth".into()], &signals);
        assert!(ranked[0].score > 50.0);
        assert!(ranked[0].explanation.symbol_match);
        assert!(ranked[0].explanation.fts_match);
        assert!(ranked[0].explanation.path_match);
    }
}
