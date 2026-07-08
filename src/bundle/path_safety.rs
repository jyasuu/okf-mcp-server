use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub enum PathSafetyError {
    ContainsDotDot(String),
    AbsolutePath(String),
    NullByte(String),
    ResolvesOutsideRoot {
        input: String,
        resolved: String,
        root: String,
    },
    SymlinkOutsideRoot {
        input: String,
        target: String,
        root: String,
    },
}

impl std::fmt::Display for PathSafetyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathSafetyError::ContainsDotDot(p) => write!(f, "path contains '..' segment: {p}"),
            PathSafetyError::AbsolutePath(p) => write!(f, "path is absolute: {p}"),
            PathSafetyError::NullByte(p) => write!(f, "path contains null byte: {p}"),
            PathSafetyError::ResolvesOutsideRoot {
                input,
                resolved,
                root,
            } => {
                write!(
                    f,
                    "path '{input}' resolves to '{resolved}' which is outside bundle root '{root}'"
                )
            }
            PathSafetyError::SymlinkOutsideRoot {
                input,
                target,
                root,
            } => {
                write!(
                    f,
                    "symlink '{input}' points to '{target}' which is outside bundle root '{root}'"
                )
            }
        }
    }
}

impl std::error::Error for PathSafetyError {}

pub struct PathChecker;

impl PathChecker {
    /// Validate that a concept_id or relative path is safe to use within the bundle root.
    /// Returns the relative path (e.g. "tables/orders.md") or an error.
    pub fn check(path: &str) -> Result<String, PathSafetyError> {
        let path = path.trim();

        if path.contains('\0') {
            return Err(PathSafetyError::NullByte(path.to_string()));
        }

        if path.starts_with('/') {
            return Err(PathSafetyError::AbsolutePath(path.to_string()));
        }

        for segment in path.split('/') {
            if segment == ".." {
                return Err(PathSafetyError::ContainsDotDot(path.to_string()));
            }
        }

        Ok(path.to_string())
    }

    /// Resolve a relative path against a root and verify it stays within the root.
    pub fn resolve(root: &Path, relative: &str) -> Result<PathBuf, PathSafetyError> {
        Self::check(relative)?;

        let candidate = root.join(relative);

        let candidate = if cfg!(target_os = "windows") {
            PathBuf::from(candidate.to_string_lossy().replace('/', "\\"))
        } else {
            candidate
        };

        let canonical = candidate.canonicalize().unwrap_or(candidate.clone());

        if !canonical.starts_with(root) {
            return Err(PathSafetyError::ResolvesOutsideRoot {
                input: relative.to_string(),
                resolved: canonical.to_string_lossy().to_string(),
                root: root.to_string_lossy().to_string(),
            });
        }

        if canonical.is_symlink() {
            let target = std::fs::read_link(&canonical).unwrap_or(canonical.clone());
            let target_canonical = if target.is_absolute() {
                target
            } else if let Some(parent) = canonical.parent() {
                parent.join(target)
            } else {
                target
            };
            if !target_canonical.starts_with(root) {
                return Err(PathSafetyError::SymlinkOutsideRoot {
                    input: relative.to_string(),
                    target: target_canonical.to_string_lossy().to_string(),
                    root: root.to_string_lossy().to_string(),
                });
            }
        }

        Ok(canonical)
    }

    /// Check if a filename is reserved (index.md or log.md).
    pub fn is_reserved_filename(path: &str) -> bool {
        let path = path.trim_end_matches(".md");
        let basename = path.rsplit('/').next().unwrap_or(path);
        basename == "index" || basename == "log"
    }

    /// Check if the path is for a root index.md.
    pub fn is_root_index(path: &str) -> bool {
        path == "index.md" || path == "index"
    }

    /// Ensure an input concept_id is safe and not reserved.
    pub fn check_concept_id(id: &str) -> Result<String, PathSafetyError> {
        let safe = Self::check(id)?;
        let path = if safe.ends_with(".md") {
            safe.clone()
        } else {
            format!("{}.md", safe)
        };
        if Self::is_reserved_filename(&path) {
            return Err(PathSafetyError::ContainsDotDot(format!(
                "concept ID uses reserved filename: {id}"
            )));
        }
        Ok(safe)
    }
}
