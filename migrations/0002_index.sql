-- Phase 2: evidence column + repository index tables

ALTER TABLE events ADD COLUMN evidence TEXT NOT NULL DEFAULT 'unknown';

CREATE TABLE IF NOT EXISTS indexed_files (
    path TEXT PRIMARY KEY NOT NULL,
    content_hash TEXT NOT NULL,
    size INTEGER NOT NULL,
    mtime REAL NOT NULL,
    language TEXT,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_indexed_files_hash ON indexed_files(content_hash);

CREATE TABLE IF NOT EXISTS symbols (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    line INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
CREATE INDEX IF NOT EXISTS idx_symbols_path ON symbols(path);

CREATE VIRTUAL TABLE IF NOT EXISTS file_chunks_fts USING fts5(
    path,
    content,
    tokenize = 'porter unicode61'
);
