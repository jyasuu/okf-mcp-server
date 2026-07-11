use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::bundle::path_safety::PathChecker;
use crate::bundle::store::{BundleStore, StoreError, StoreResult};

pub struct LocalFsStore {
    root: PathBuf,
    write_mutex: Mutex<()>,
}

impl LocalFsStore {
    pub fn new(root: PathBuf) -> Self {
        let canonical = root.canonicalize().unwrap_or(root);
        Self {
            root: canonical,
            write_mutex: Mutex::new(()),
        }
    }

    fn resolve(&self, path: &str) -> StoreResult<PathBuf> {
        let safe = PathChecker::check(path)?;
        let resolved = self.root.join(&safe);
        if let Ok(canonical) = resolved.canonicalize() {
            if !canonical.starts_with(&self.root) {
                return Err(StoreError::PathSafety(
                    crate::bundle::path_safety::PathSafetyError::ResolvesOutsideRoot {
                        input: path.to_string(),
                        resolved: canonical.to_string_lossy().to_string(),
                        root: self.root.to_string_lossy().to_string(),
                    },
                ));
            }
            Ok(canonical)
        } else {
            // File doesn't exist yet — verify each existing parent is safe
            self.verify_parent_chain(&safe)?;
            Ok(resolved)
        }
    }

    fn verify_parent_chain(&self, relative: &str) -> StoreResult<()> {
        let parts: Vec<&str> = relative.split('/').collect();
        let mut current = self.root.clone();
        for part in &parts[..parts.len().saturating_sub(1)] {
            current = current.join(part);
            if current.is_symlink() {
                let target = std::fs::read_link(&current).map_err(StoreError::Io)?;
                let canonical = if target.is_absolute() {
                    target
                } else {
                    current.parent().unwrap_or(&self.root).join(target)
                };
                if !canonical.starts_with(&self.root) {
                    return Err(StoreError::PathSafety(
                        crate::bundle::path_safety::PathSafetyError::SymlinkOutsideRoot {
                            input: relative.to_string(),
                            target: canonical.to_string_lossy().to_string(),
                            root: self.root.to_string_lossy().to_string(),
                        },
                    ));
                }
            }
        }
        Ok(())
    }
}

impl BundleStore for LocalFsStore {
    fn list_files(&self, prefix: Option<&str>) -> StoreResult<Vec<String>> {
        if let Some(p) = prefix {
            PathChecker::check(p).map_err(|e| StoreError::Other(e.to_string()))?;
        }

        let mut files = Vec::new();
        collect_md_files(&self.root, &self.root, &mut files)?;

        // Filter by prefix string
        let files = if let Some(pref) = prefix {
            let pref_norm = pref.trim_end_matches('/');
            files
                .into_iter()
                .filter(|f| f.starts_with(pref_norm))
                .collect()
        } else {
            files
        };

        Ok(files)
    }

    fn read_raw(&self, path: &str) -> StoreResult<String> {
        let resolved = self.resolve(path)?;
        if !resolved.exists() {
            return Err(StoreError::NotFound(path.to_string()));
        }
        std::fs::read_to_string(&resolved).map_err(StoreError::Io)
    }

    fn write_raw(&self, path: &str, content: &str) -> StoreResult<()> {
        let _guard = self
            .write_mutex
            .lock()
            .map_err(|e| StoreError::Other(format!("lock poisoned: {e}")))?;
        let resolved = self.resolve(path)?;

        // Ensure parent directory exists
        if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent).map_err(StoreError::Io)?;
        }

        // Atomic write: write to temp file, then rename
        let tmp_path = resolved.with_extension("tmp");
        std::fs::write(&tmp_path, content).map_err(StoreError::Io)?;
        std::fs::rename(&tmp_path, &resolved).map_err(StoreError::Io)?;
        Ok(())
    }

    fn delete_raw(&self, path: &str) -> StoreResult<()> {
        let _guard = self
            .write_mutex
            .lock()
            .map_err(|e| StoreError::Other(format!("lock poisoned: {e}")))?;
        let resolved = self.resolve(path)?;
        if !resolved.exists() {
            return Err(StoreError::NotFound(path.to_string()));
        }
        std::fs::remove_file(&resolved).map_err(StoreError::Io)
    }

    fn exists(&self, path: &str) -> bool {
        match self.resolve(path) {
            Ok(p) => p.exists(),
            Err(_) => false,
        }
    }
}

fn collect_md_files(
    dir: &Path,
    root: &Path,
    files: &mut Vec<String>,
) -> Result<(), std::io::Error> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_md_files(&path, root, files)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let rel = pathdiff::diff_paths(&path, root);
            if let Some(rel) = rel {
                files.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    Ok(())
}
