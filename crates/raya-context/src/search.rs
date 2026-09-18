//! Keyword extraction and lexical search.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use grep_matcher::Matcher;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{Searcher, sinks};
use ignore::WalkBuilder;

use crate::rank::RankedCandidate;

const STOP: &[&str] = &[
    "a", "an", "the", "to", "and", "or", "of", "in", "on", "for", "with", "is", "are", "be",
    "this", "that", "it", "as", "at", "by", "from", "add", "update", "fix", "please",
];

pub fn extract_keywords(request: &str) -> Vec<String> {
    let mut words: Vec<String> = request
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_ascii_lowercase())
        .filter(|w| !STOP.contains(&w.as_str()))
        .collect();
    words.sort();
    words.dedup();
    words
}

pub fn search_candidates(
    root: &Path,
    keywords: &[String],
    max_files: usize,
) -> Vec<RankedCandidate> {
    if keywords.is_empty() {
        return Vec::new();
    }

    let pattern = keywords
        .iter()
        .map(|k| regex_escape(k))
        .collect::<Vec<_>>()
        .join("|");

    let matcher = match RegexMatcherBuilder::new()
        .case_insensitive(true)
        .build(&pattern)
    {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    let hits: Arc<Mutex<BTreeMap<String, (usize, Vec<String>)>>> =
        Arc::new(Mutex::new(BTreeMap::new()));
    let mut searcher = Searcher::new();
    let mut builder = WalkBuilder::new(root);
    builder.hidden(false).git_ignore(true);

    for entry in builder.build().filter_map(|e| e.ok()) {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path().to_path_buf();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();

        // Skip huge / binary-ish files by extension
        if is_skipped(&rel) {
            continue;
        }

        let hits2 = hits.clone();
        let matcher2 = matcher.clone();
        let _ = searcher.search_path(
            &matcher2,
            &path,
            sinks::UTF8(|_line_num, line| {
                if matcher2.find(line.as_bytes()).ok().flatten().is_some() {
                    let mut guard = hits2.lock().unwrap_or_else(|e| e.into_inner());
                    let entry = guard.entry(rel.clone()).or_insert_with(|| (0, Vec::new()));
                    entry.0 += 1;
                    if entry.1.len() < 5 {
                        entry.1.push(line.trim().to_string());
                    }
                }
                Ok(true)
            }),
        );
    }

    let guard = hits.lock().unwrap_or_else(|e| e.into_inner());
    let mut candidates: Vec<RankedCandidate> = guard
        .iter()
        .map(|(path, (count, snippets))| {
            let size = std::fs::metadata(root.join(path))
                .map(|m| m.len())
                .unwrap_or(0);
            RankedCandidate {
                path: PathBuf::from(path),
                hit_count: *count,
                size_bytes: size,
                preview_lines: snippets.clone(),
                score: 0.0,
                explanation: Default::default(),
            }
        })
        .collect();

    // Also boost path/name keyword matches even without content hits
    for kw in keywords {
        let mut builder = WalkBuilder::new(root);
        builder.hidden(false).git_ignore(true);
        for entry in builder.build().filter_map(|e| e.ok()) {
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap_or(path)
                .display()
                .to_string()
                .to_ascii_lowercase();
            if rel.contains(kw) && !candidates.iter().any(|c| c.path.to_string_lossy() == rel) {
                let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                candidates.push(RankedCandidate {
                    path: PathBuf::from(
                        path.strip_prefix(root)
                            .unwrap_or(path)
                            .display()
                            .to_string(),
                    ),
                    hit_count: 0,
                    size_bytes: size,
                    preview_lines: Vec::new(),
                    score: 0.0,
                    explanation: Default::default(),
                });
            }
        }
    }

    candidates.truncate(max_files.saturating_mul(3).max(max_files));
    candidates
}

fn regex_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if matches!(
            c,
            '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$' | '\\'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn is_skipped(rel: &str) -> bool {
    let lower = rel.to_ascii_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".lock")
        || lower.contains("target/")
        || lower.contains("node_modules/")
        || lower.contains(".git/")
}
