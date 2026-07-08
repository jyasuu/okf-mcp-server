use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::bundle::repo::BundleRepo;
use crate::bundle::types::ConceptId;

pub struct FileWatcher {
    repos: HashMap<String, Arc<BundleRepo>>,
}

impl FileWatcher {
    pub fn new(repos: HashMap<String, Arc<BundleRepo>>) -> Self {
        Self { repos }
    }

    pub fn start(&self) -> Result<(), String> {
        let (tx, rx) = std::sync::mpsc::channel::<Result<Event, notify::Error>>();

        let mut watcher: RecommendedWatcher =
            Watcher::new(tx, Config::default().with_poll_interval(Duration::from_secs(2)))
                .map_err(|e| format!("failed to create file watcher: {e}"))?;

        for (name, repo) in &self.repos {
            let root = repo.root().to_path_buf();
            if root.exists() {
                watcher
                    .watch(&root, RecursiveMode::Recursive)
                    .map_err(|e| format!("failed to watch bundle '{name}' at {root:?}: {e}"))?;
                tracing::info!("watching bundle '{name}' at {root:?}");
            } else {
                tracing::warn!("bundle '{name}' root {root:?} does not exist, skipping watch");
            }
        }

        // Spawn a thread to process events
        let repos = self.repos.clone();
        std::thread::spawn(move || {
            let mut pending: HashMap<PathBuf, EventKind> = HashMap::new();
            let mut last_event: Option<std::time::Instant> = None;

            loop {
                // Process available events with debouncing
                while let Ok(result) = rx.try_recv() {
                    match result {
                        Ok(event) => {
                            for path in &event.paths {
                                if should_ignore(path) {
                                    continue;
                                }
                                pending.insert(path.clone(), event.kind.clone());
                                last_event = Some(std::time::Instant::now());
                            }
                        }
                        Err(e) => {
                            tracing::error!("file watcher error: {e}");
                        }
                    }
                }

                // Process pending events after debounce period
                if let Some(last) = last_event {
                    if last.elapsed() >= Duration::from_millis(300) {
                        process_events(&repos, &pending);
                        pending.clear();
                        last_event = None;
                    }
                }

                std::thread::sleep(Duration::from_millis(100));
            }
        });

        Ok(())
    }
}

fn should_ignore(path: &Path) -> bool {
    let name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return true,
    };

    // Ignore hidden files, temp files, non-md files
    if name.starts_with('.') {
        return true;
    }
    if name.starts_with('#') || name.ends_with('~') || name.ends_with(".swp") || name.ends_with(".swx") {
        return true;
    }

    // Only watch .md files (concept files and index.md/log.md)
    if !name.ends_with(".md") {
        return true;
    }

    false
}

fn process_events(repos: &HashMap<String, Arc<BundleRepo>>, pending: &HashMap<PathBuf, EventKind>) {
    for (path, kind) in pending {
        // Find which bundle this path belongs to
        let (_, repo) = match repos.iter().find(|(_, r)| path.starts_with(r.root())) {
            Some((name, repo)) => (name.clone(), repo),
            None => continue,
        };

        // Compute the relative path within the bundle
        let rel = match path.strip_prefix(repo.root()) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let rel_str = rel.to_string_lossy().replace('\\', "/");

        // Determine the concept ID from the relative path
        let concept_id = match rel_str.strip_suffix(".md") {
            Some(id) => ConceptId::new(id),
            None => continue,
        };

        match kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                tracing::debug!(
                    "file change detected: {rel_str} ({kind:?}), re-indexing"
                );
                repo.reindex_concept(&concept_id);
            }
            EventKind::Remove(_) => {
                tracing::debug!(
                    "file deletion detected: {rel_str}, removing from index"
                );
                repo.remove_from_index(&concept_id);
            }
            _ => {}
        }
    }
}
