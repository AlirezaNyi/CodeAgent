//! Incremental repository indexer.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod symbols;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::Utc;
use ignore::WalkBuilder;
use raya_store::{Store, StoreError};
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};
use tracing::info;

pub use symbols::{SymbolHit, extract_symbols};

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error(transparent)]
    Store(#[from] StoreError),

    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Message(String),
}

pub type IndexResult<T> = Result<T, IndexError>;

#[derive(Debug, Clone, Default)]
pub struct IndexStats {
    pub scanned: u64,
    pub updated: u64,
    pub unchanged: u64,
    pub removed: u64,
    pub symbols: u64,
}

/// Run an incremental index of `root` into the store's SQLite DB.
pub fn index_project(store: &Store, root: &Path) -> IndexResult<IndexStats> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

    let mut stats = IndexStats::default();
    let mut seen: Vec<String> = Vec::new();

    let walker = WalkBuilder::new(&root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        if should_skip(&rel) {
            continue;
        }

        stats.scanned += 1;
        let meta = match fs::metadata(path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let size = meta.len();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);

        let existing: Option<(String, i64, f64)> = store.with_conn(|conn| {
            conn.query_row(
                "SELECT content_hash, size, mtime FROM indexed_files WHERE path = ?1",
                params![rel],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(StoreError::from)
        })?;

        if let Some((_hash, old_size, old_mtime)) = &existing
            && *old_size == size as i64
            && (*old_mtime - mtime).abs() < f64::EPSILON
        {
            stats.unchanged += 1;
            seen.push(rel);
            continue;
        }

        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        // Skip likely-binary
        if bytes.iter().take(1024).any(|&b| b == 0) {
            continue;
        }
        let hash = hex::encode(Sha256::digest(&bytes));
        if let Some((old_hash, _, _)) = &existing
            && old_hash == &hash
        {
            stats.unchanged += 1;
            seen.push(rel);
            continue;
        }

        let text = String::from_utf8_lossy(&bytes);
        let language = detect_language(&rel);
        let now = Utc::now().to_rfc3339();
        let syms = extract_symbols(&text, language);

        store.with_conn(|conn| {
            conn.execute(
                "INSERT INTO indexed_files (path, content_hash, size, mtime, language, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(path) DO UPDATE SET
                    content_hash = excluded.content_hash,
                    size = excluded.size,
                    mtime = excluded.mtime,
                    language = excluded.language,
                    updated_at = excluded.updated_at",
                params![rel, hash, size as i64, mtime, language, now],
            )?;

            conn.execute("DELETE FROM symbols WHERE path = ?1", params![rel])?;
            for s in &syms {
                conn.execute(
                    "INSERT INTO symbols (path, name, kind, line, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![rel, s.name, s.kind, s.line as i64, now],
                )?;
            }

            // FTS: delete old rows for path then insert chunk(s)
            conn.execute("DELETE FROM file_chunks_fts WHERE path = ?1", params![rel])?;
            let chunk = truncate_for_fts(&text, 80_000);
            conn.execute(
                "INSERT INTO file_chunks_fts (path, content) VALUES (?1, ?2)",
                params![rel, chunk],
            )?;
            Ok(())
        })?;

        stats.updated += 1;
        stats.symbols += syms.len() as u64;
        seen.push(rel);
    }

    // Remove deleted files
    let indexed: Vec<String> = store.with_conn(|conn| {
        let mut stmt = conn.prepare("SELECT path FROM indexed_files")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    })?;

    for path in indexed {
        if !seen.iter().any(|s| s == &path) {
            store.with_conn(|conn| {
                conn.execute("DELETE FROM indexed_files WHERE path = ?1", params![path])?;
                conn.execute("DELETE FROM symbols WHERE path = ?1", params![path])?;
                conn.execute("DELETE FROM file_chunks_fts WHERE path = ?1", params![path])?;
                Ok(())
            })?;
            stats.removed += 1;
        }
    }

    store.set_index_meta("last_index_at", &Utc::now().to_rfc3339())?;
    store.set_index_meta("last_index_count", &stats.scanned.to_string())?;
    store.set_index_meta("index_version", "2")?;

    info!(
        scanned = stats.scanned,
        updated = stats.updated,
        unchanged = stats.unchanged,
        removed = stats.removed,
        "index complete"
    );
    Ok(stats)
}

/// FTS5 search returning matching paths (best effort).
pub fn fts_search(store: &Store, query: &str, limit: usize) -> IndexResult<Vec<String>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    // Escape FTS special chars loosely by quoting tokens
    let q = query
        .split_whitespace()
        .map(|t| {
            let clean: String = t
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if clean.is_empty() {
                String::new()
            } else {
                format!("\"{clean}\"")
            }
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" OR ");
    if q.is_empty() {
        return Ok(Vec::new());
    }

    store
        .with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT path FROM file_chunks_fts WHERE file_chunks_fts MATCH ?1 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![q, limit as i64], |row| row.get(0))?;
            let mut out = Vec::new();
            for r in rows {
                match r {
                    Ok(p) => out.push(p),
                    Err(_) => break,
                }
            }
            Ok(out)
        })
        .map_err(IndexError::from)
}

/// Symbol name lookup.
pub fn find_symbols(store: &Store, name_substr: &str, limit: usize) -> IndexResult<Vec<SymbolHit>> {
    let pattern = format!("%{name_substr}%");
    store
        .with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT path, name, kind, line FROM symbols
             WHERE name LIKE ?1 COLLATE NOCASE
             LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![pattern, limit as i64], |row| {
                Ok(SymbolHit {
                    path: PathBuf::from(row.get::<_, String>(0)?),
                    name: row.get(1)?,
                    kind: row.get(2)?,
                    line: row.get::<_, i64>(3)? as u32,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
        .map_err(IndexError::from)
}

fn should_skip(rel: &str) -> bool {
    let l = rel.to_ascii_lowercase();
    l.starts_with(".git/")
        || l.contains("/target/")
        || l.starts_with("target/")
        || l.contains("/node_modules/")
        || l.starts_with("node_modules/")
        || l.ends_with(".png")
        || l.ends_with(".jpg")
        || l.ends_with(".webp")
        || l.ends_with(".wasm")
        || l.ends_with(".lock")
        || l.ends_with(".raya/raya.db")
        || l.contains(".raya/raya.db")
}

fn detect_language(path: &str) -> &'static str {
    let l = path.to_ascii_lowercase();
    if l.ends_with(".rs") {
        "rust"
    } else if l.ends_with(".ts") || l.ends_with(".tsx") {
        "typescript"
    } else if l.ends_with(".js") || l.ends_with(".jsx") {
        "javascript"
    } else if l.ends_with(".py") {
        "python"
    } else if l.ends_with(".go") {
        "go"
    } else if l.ends_with(".java") {
        "java"
    } else {
        "text"
    }
}

fn truncate_for_fts(text: &str, max: usize) -> String {
    if text.len() <= max {
        text.to_string()
    } else {
        text.chars().take(max).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn incremental_index_and_fts() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("hello.rs"), "fn hello_raya() {}\n").unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        let stats = index_project(&store, dir.path()).unwrap();
        assert!(stats.updated >= 1);

        let stats2 = index_project(&store, dir.path()).unwrap();
        assert!(stats2.unchanged >= 1);

        let hits = fts_search(&store, "hello_raya", 10).unwrap();
        assert!(hits.iter().any(|p| p.contains("hello.rs")), "{hits:?}");

        let syms = find_symbols(&store, "hello_raya", 10).unwrap();
        assert!(!syms.is_empty());
    }
}
