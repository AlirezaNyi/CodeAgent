//! Project-root path confinement.

use std::path::{Component, Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PathError {
    #[error("path escapes project root: {0}")]
    Traversal(String),

    #[error("I/O error resolving path: {0}")]
    Io(#[from] std::io::Error),
}

/// Resolve `user_path` relative to `project_root`, rejecting traversal escapes.
pub fn safe_path(project_root: &Path, user_path: &str) -> Result<PathBuf, PathError> {
    let root = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());

    let normalized = normalize_lexically(&root, user_path)?;
    if !normalized.starts_with(&root) {
        return Err(PathError::Traversal(user_path.to_string()));
    }

    // If the path exists, also check symlink resolution.
    if normalized.exists() {
        let canon = normalized.canonicalize()?;
        if !canon.starts_with(&root) {
            return Err(PathError::Traversal(user_path.to_string()));
        }
        return Ok(canon);
    }

    Ok(normalized)
}

fn normalize_lexically(root: &Path, user_path: &str) -> Result<PathBuf, PathError> {
    let joined = if Path::new(user_path).is_absolute() {
        PathBuf::from(user_path)
    } else {
        root.join(user_path)
    };

    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                if !out.pop() {
                    return Err(PathError::Traversal(user_path.to_string()));
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn accepts_relative() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        let p = safe_path(dir.path(), "a.txt").unwrap();
        assert!(p.ends_with("a.txt"));
    }

    #[test]
    fn rejects_traversal() {
        let dir = tempdir().unwrap();
        let err = safe_path(dir.path(), "../outside.txt");
        assert!(err.is_err());
    }

    #[test]
    fn rejects_nested_traversal() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        let err = safe_path(dir.path(), "sub/../../outside.txt");
        assert!(err.is_err());
    }
}
