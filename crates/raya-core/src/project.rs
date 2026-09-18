//! Project root discovery.

use std::path::{Path, PathBuf};

use crate::error::{ProjectError, Result};

/// Absolute path to a discovered project root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRoot {
    path: PathBuf,
}

impl ProjectRoot {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn into_path(self) -> PathBuf {
        self.path
    }
}

/// Walk upward from `start` (or cwd) looking for `.git` or `.raya`.
pub fn discover(start: Option<&Path>) -> Result<ProjectRoot> {
    let start = match start {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir().map_err(ProjectError::Cwd)?,
    };

    let mut current = if start.is_file() {
        start.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        start.clone()
    };

    loop {
        if current.join(".git").exists() || current.join(".raya").exists() {
            return Ok(ProjectRoot::new(
                current.canonicalize().unwrap_or_else(|_| current.clone()),
            ));
        }
        if !current.pop() {
            break;
        }
    }

    Err(ProjectError::NotFound {
        start: start.display().to_string(),
    }
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn finds_git_root() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let nested = dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        let root = discover(Some(&nested)).unwrap();
        assert_eq!(
            root.path().canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn finds_raya_dir() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".raya")).unwrap();
        let root = discover(Some(dir.path())).unwrap();
        assert_eq!(
            root.path().canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn not_found() {
        let dir = tempdir().unwrap();
        let err = discover(Some(dir.path()));
        assert!(err.is_err());
    }
}
