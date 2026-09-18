//! Relevance ranking with explanations.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RankExplanation {
    pub lexical_hits: usize,
    pub path_match: bool,
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
}

pub fn rank(mut candidates: Vec<RankedCandidate>, keywords: &[String]) -> Vec<RankedCandidate> {
    for c in &mut candidates {
        let path_l = c.path.to_string_lossy().to_ascii_lowercase();
        let path_match = keywords.iter().any(|k| path_l.contains(k));
        let lexical = c.hit_count as f64 * 10.0;
        let path_score = if path_match { 25.0 } else { 0.0 };
        let size_penalty = (c.size_bytes as f64 / 10_000.0).min(20.0);
        let score = lexical + path_score - size_penalty;

        let mut notes = Vec::new();
        if c.hit_count > 0 {
            notes.push(format!("{} lexical hits", c.hit_count));
        }
        if path_match {
            notes.push("path/name matched keywords".into());
        }
        if size_penalty > 0.0 {
            notes.push(format!("size penalty {size_penalty:.1}"));
        }

        c.score = score;
        c.explanation = RankExplanation {
            lexical_hits: c.hit_count,
            path_match,
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
