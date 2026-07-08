use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use git2::{FetchOptions, PushOptions, RemoteCallbacks, Repository, Signature};

use crate::bundle::path_safety::PathChecker;
use crate::bundle::store::{
    BundleStore, GitControl, GitStatus, PullResult, PushResult, StoreError, StoreResult,
};

pub struct GitStore {
    root: PathBuf,
    repo: Mutex<Repository>,
    write_mutex: Mutex<()>,
    ssh_key_path: Option<String>,
    token_env: Option<String>,
}

impl GitStore {
    pub fn new(
        root: PathBuf,
        ssh_key_path: Option<String>,
        token_env: Option<String>,
    ) -> StoreResult<Self> {
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        let repo = Repository::open(&canonical)
            .or_else(|_| Repository::init(&canonical))
            .map_err(|e| StoreError::Other(format!("failed to open/init git repo: {e}")))?;
        Ok(Self {
            root: canonical,
            repo: Mutex::new(repo),
            write_mutex: Mutex::new(()),
            ssh_key_path,
            token_env,
        })
    }

    fn resolve(&self, path: &str) -> StoreResult<PathBuf> {
        let safe = PathChecker::check(path)?;
        let resolved = self.root.join(&safe);
        let canonical = resolved.canonicalize().unwrap_or(resolved);
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
    }

    fn relative_path(&self, abs: &Path) -> String {
        let rel = pathdiff::diff_paths(abs, &self.root);
        let rel = rel.unwrap_or_else(|| abs.to_path_buf());
        rel.to_string_lossy().replace('\\', "/")
    }

    fn stage_path(&self, path: &str) -> StoreResult<()> {
        let rel = path.trim_start_matches('/');
        let repo = self.repo.lock().unwrap();
        let mut index = repo
            .index()
            .map_err(|e| StoreError::Other(format!("failed to open index: {e}")))?;
        index
            .add_path(Path::new(rel))
            .map_err(|e| StoreError::Other(format!("failed to stage {rel}: {e}")))?;
        index
            .write()
            .map_err(|e| StoreError::Other(format!("failed to write index: {e}")))?;
        Ok(())
    }

    fn callbacks(&self) -> RemoteCallbacks<'static> {
        let ssh_key_path = self.ssh_key_path.clone();
        let token_env = self.token_env.clone();
        let mut callbacks = RemoteCallbacks::new();

        callbacks.credentials(move |_url, username_from_url, allowed_types| {
            if allowed_types.contains(git2::CredentialType::SSH_KEY) {
                if let Some(ref key_path) = ssh_key_path {
                    let username = username_from_url.unwrap_or("git");
                    return git2::Cred::ssh_key(username, None, Path::new(key_path), None);
                }
            }
            if allowed_types.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
                if let Some(ref env_var) = token_env {
                    if let Ok(token) = std::env::var(env_var) {
                        return git2::Cred::userpass_plaintext("x-access-token", &token);
                    }
                }
            }
            if allowed_types.contains(git2::CredentialType::DEFAULT) {
                return git2::Cred::default();
            }
            Err(git2::Error::from_str("no credentials available"))
        });

        callbacks
    }
}

impl BundleStore for GitStore {
    fn list_files(&self, prefix: Option<&str>) -> StoreResult<Vec<String>> {
        if let Some(p) = prefix {
            PathChecker::check(p).map_err(|e| StoreError::Other(e.to_string()))?;
        }

        let mut files = Vec::new();
        {
            let repo = self.repo.lock().unwrap();
            let tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());

            if let Some(tree) = tree {
                walk_tree(&repo, &tree, "", &mut files)?;
            } else {
                collect_md_files(&self.root, &self.root, &mut files)?;
            }
        }

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
        let _guard = self.write_mutex.lock().unwrap();
        let resolved = self.resolve(path)?;

        if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent).map_err(StoreError::Io)?;
        }

        let tmp_path = resolved.with_extension("tmp");
        std::fs::write(&tmp_path, content).map_err(StoreError::Io)?;
        std::fs::rename(&tmp_path, &resolved).map_err(StoreError::Io)?;

        self.stage_path(path)?;

        Ok(())
    }

    fn delete_raw(&self, path: &str) -> StoreResult<()> {
        let _guard = self.write_mutex.lock().unwrap();
        let resolved = self.resolve(path)?;
        if !resolved.exists() {
            return Err(StoreError::NotFound(path.to_string()));
        }
        std::fs::remove_file(&resolved).map_err(StoreError::Io)?;

        let rel = path.trim_start_matches('/');
        let repo = self.repo.lock().unwrap();
        let mut index = repo
            .index()
            .map_err(|e| StoreError::Other(format!("failed to open index: {e}")))?;
        index
            .remove_path(Path::new(rel))
            .map_err(|e| StoreError::Other(format!("failed to stage deletion: {e}")))?;
        index
            .write()
            .map_err(|e| StoreError::Other(format!("failed to write index: {e}")))?;

        Ok(())
    }

    fn exists(&self, path: &str) -> bool {
        match self.resolve(path) {
            Ok(p) => p.exists(),
            Err(_) => false,
        }
    }
}

impl GitControl for GitStore {
    fn status(&self) -> StoreResult<GitStatus> {
        let repo = self.repo.lock().unwrap();
        let branch = repo
            .head()
            .ok()
            .and_then(|h| h.shorthand().map(String::from))
            .unwrap_or_else(|| "HEAD".to_string());

        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        let mut untracked = Vec::new();

        let mut index = repo
            .index()
            .map_err(|e| StoreError::Other(format!("failed to open index: {e}")))?;
        let diff_staged = repo
            .diff_tree_to_index(None, Some(&index), None)
            .map_err(|e| StoreError::Other(format!("failed to diff: {e}")))?;
        diff_staged
            .foreach(
                &mut |delta, _progress| {
                    if let Some(file) = delta.new_file().path() {
                        staged.push(file.to_string_lossy().to_string());
                    }
                    true
                },
                None,
                None,
                None,
            )
            .map_err(|e| StoreError::Other(format!("failed to iterate staged diff: {e}")))?;

        let diff_unstaged = repo
            .diff_index_to_workdir(Some(&index), None)
            .map_err(|e| StoreError::Other(format!("failed to diff workdir: {e}")))?;
        diff_unstaged
            .foreach(
                &mut |delta, _progress| {
                    if let Some(file) = delta.new_file().path() {
                        unstaged.push(file.to_string_lossy().to_string());
                    }
                    true
                },
                None,
                None,
                None,
            )
            .map_err(|e| StoreError::Other(format!("failed to iterate unstaged diff: {e}")))?;

        let statuses = repo
            .statuses(Some(
                git2::StatusOptions::new()
                    .include_untracked(true)
                    .recurse_untracked_dirs(true),
            ))
            .map_err(|e| StoreError::Other(format!("failed to get status: {e}")))?;
        for entry in statuses.iter() {
            if entry.status() == git2::Status::WT_NEW {
                if let Some(path) = entry.path() {
                    untracked.push(path.to_string());
                }
            }
        }

        Ok(GitStatus {
            staged,
            unstaged,
            untracked,
            branch,
        })
    }

    fn diff(&self, path: Option<&str>) -> StoreResult<String> {
        let repo = self.repo.lock().unwrap();
        let mut index = repo
            .index()
            .map_err(|e| StoreError::Other(format!("failed to open index: {e}")))?;
        let diff = if let Some(single_path) = path {
            let mut opts = git2::DiffOptions::new();
            opts.pathspec(single_path);
            repo.diff_index_to_workdir(Some(&index), Some(&mut opts))
        } else {
            repo.diff_index_to_workdir(Some(&index), None)
        }
        .map_err(|e| StoreError::Other(format!("failed to diff: {e}")))?;

        let mut buf = Vec::new();
        diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
            let origin = match line.origin() {
                '+' => '+',
                '-' => '-',
                ' ' => ' ',
                _ => line.origin(),
            };
            let content = std::str::from_utf8(line.content()).unwrap_or("");
            let _ = write!(buf, "{}{}", origin, content);
            true
        })
        .map_err(|e| StoreError::Other(format!("failed to print diff: {e}")))?;

        String::from_utf8(buf)
            .map_err(|e| StoreError::Other(format!("diff is not valid utf-8: {e}")))
    }

    fn commit(&self, message: &str, author: Option<&str>) -> StoreResult<String> {
        let repo = self.repo.lock().unwrap();
        let sig = if let Some(name) = author {
            Signature::now(name, "agent@okf-mcp-server")
        } else {
            Signature::now("okf-mcp-server", "agent@okf-mcp-server")
        }
        .map_err(|e| StoreError::Other(format!("failed to create signature: {e}")))?;

        let mut index = repo
            .index()
            .map_err(|e| StoreError::Other(format!("failed to open index: {e}")))?;
        let tree_oid = index
            .write_tree()
            .map_err(|e| StoreError::Other(format!("failed to write tree: {e}")))?;
        let tree = repo
            .find_tree(tree_oid)
            .map_err(|e| StoreError::Other(format!("failed to find tree: {e}")))?;

        let parent_commit = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent_commit.iter().collect();

        let commit_oid = if parents.is_empty() {
            repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[])
        } else {
            repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
        }
        .map_err(|e| StoreError::Other(format!("failed to commit: {e}")))?;

        Ok(commit_oid.to_string())
    }

    fn push(&self, remote: Option<&str>, branch: Option<&str>) -> StoreResult<PushResult> {
        let repo = self.repo.lock().unwrap();
        let remote_name = remote.unwrap_or("origin");
        let mut remote_obj = repo
            .find_remote(remote_name)
            .map_err(|e| StoreError::Other(format!("remote not found '{remote_name}': {e}")))?;

        let branch_ref = match branch {
            Some(b) => format!("refs/heads/{b}"),
            None => {
                let head = repo
                    .head()
                    .map_err(|e| StoreError::Other(format!("no HEAD: {e}")))?;
                head.name()
                    .ok_or_else(|| StoreError::Other("HEAD has no name".to_string()))?
                    .to_string()
            }
        };

        let branch_short = branch_ref
            .strip_prefix("refs/heads/")
            .unwrap_or(&branch_ref);

        let mut push_opts = PushOptions::new();
        let callbacks = self.callbacks();
        push_opts.remote_callbacks(callbacks);

        remote_obj
            .push(&[&branch_ref], Some(&mut push_opts))
            .map_err(|e| StoreError::Other(format!("push failed: {e}")))?;

        Ok(PushResult {
            pushed_branch: branch_short.to_string(),
            remote: remote_name.to_string(),
        })
    }

    fn pull(&self, remote: Option<&str>, branch: Option<&str>) -> StoreResult<PullResult> {
        let remote_name = remote.unwrap_or("origin");
        let repo = self.repo.lock().unwrap();

        let branch_name = branch
            .map(|s| s.to_string())
            .or_else(|| {
                repo.head()
                    .ok()
                    .and_then(|h| h.shorthand().map(|s| s.to_string()))
            })
            .unwrap_or_else(|| "main".to_string());

        let mut remote_obj = repo
            .find_remote(remote_name)
            .map_err(|e| StoreError::Other(format!("remote not found '{remote_name}': {e}")))?;

        let mut fetch_opts = FetchOptions::new();
        let callbacks = self.callbacks();
        fetch_opts.remote_callbacks(callbacks);

        let refspecs: &[&str] = &[];
        remote_obj
            .fetch(refspecs, Some(&mut fetch_opts), None)
            .map_err(|e| StoreError::Other(format!("fetch failed: {e}")))?;

        let fetch_head = repo
            .find_reference("FETCH_HEAD")
            .map_err(|e| StoreError::Other(format!("FETCH_HEAD not found: {e}")))?;
        let fetch_commit = fetch_head
            .peel_to_commit()
            .map_err(|e| StoreError::Other(format!("failed to peel FETCH_HEAD: {e}")))?;

        let branch_ref_name = format!("refs/heads/{branch_name}");
        let branch_ref = repo.find_reference(&branch_ref_name).ok();

        if let Some(mut local_ref) = branch_ref {
            let local_commit = local_ref
                .peel_to_commit()
                .map_err(|e| StoreError::Other(format!("failed to peel local ref: {e}")))?;

            let merge_base = repo.merge_base(local_commit.id(), fetch_commit.id());

            if let Ok(base_oid) = merge_base {
                if base_oid == fetch_commit.id() {
                    return Ok(PullResult {
                        updated: false,
                        conflicts: None,
                    });
                }
                if base_oid == local_commit.id() {
                    // Fast-forward
                    let annotated = repo.find_annotated_commit(fetch_commit.id()).map_err(|e| {
                        StoreError::Other(format!("failed to annotate commit: {e}"))
                    })?;
                    let analysis = repo
                        .merge_analysis(&[&annotated])
                        .map_err(|e| StoreError::Other(format!("merge analysis failed: {e}")))?;

                    if analysis.0.is_up_to_date() {
                        return Ok(PullResult {
                            updated: false,
                            conflicts: None,
                        });
                    }

                    if analysis.0.is_fast_forward() {
                        let fetch_tree = fetch_commit.tree().map_err(|e| {
                            StoreError::Other(format!("failed to get fetch tree: {e}"))
                        })?;

                        let mut checkout = git2::build::CheckoutBuilder::new();
                        checkout.force();
                        repo.checkout_tree(fetch_tree.as_object(), Some(&mut checkout))
                            .map_err(|e| StoreError::Other(format!("checkout failed: {e}")))?;

                        local_ref
                            .set_target(fetch_commit.id(), "pull: fast-forward")
                            .map_err(|e| StoreError::Other(format!("failed to update ref: {e}")))?;
                        return Ok(PullResult {
                            updated: true,
                            conflicts: None,
                        });
                    }
                }
            }

            // Not a fast-forward, check for conflicts
            let annotated = repo
                .find_annotated_commit(fetch_commit.id())
                .map_err(|e| StoreError::Other(format!("failed to annotate commit: {e}")))?;
            let analysis = repo
                .merge_analysis(&[&annotated])
                .map_err(|e| StoreError::Other(format!("merge analysis failed: {e}")))?;

            if analysis.0.is_up_to_date() {
                return Ok(PullResult {
                    updated: false,
                    conflicts: None,
                });
            }

            if analysis.0.is_fast_forward() {
                let fetch_tree = fetch_commit
                    .tree()
                    .map_err(|e| StoreError::Other(format!("failed to get fetch tree: {e}")))?;

                let mut checkout = git2::build::CheckoutBuilder::new();
                checkout.force();
                repo.checkout_tree(fetch_tree.as_object(), Some(&mut checkout))
                    .map_err(|e| StoreError::Other(format!("checkout failed: {e}")))?;

                local_ref
                    .set_target(fetch_commit.id(), "pull: fast-forward")
                    .map_err(|e| StoreError::Other(format!("failed to update ref: {e}")))?;
                return Ok(PullResult {
                    updated: true,
                    conflicts: None,
                });
            }

            if analysis.0.is_normal() {
                repo.set_head(&branch_ref_name)
                    .map_err(|e| StoreError::Other(format!("failed to set head: {e}")))?;

                let annotated = repo
                    .find_annotated_commit(fetch_commit.id())
                    .map_err(|e| StoreError::Other(format!("failed to annotate commit: {e}")))?;
                let merge_result = repo.merge(&[&annotated], None, None);

                match merge_result {
                    Ok(()) => {
                        let mut index = repo
                            .index()
                            .map_err(|e| StoreError::Other(format!("failed to get index: {e}")))?;

                        if index.has_conflicts() {
                            let mut conflicts = Vec::new();
                            let paths: Vec<Vec<u8>> = index
                                .conflicts()
                                .map_err(|e| {
                                    StoreError::Other(format!("failed to get conflicts: {e}"))
                                })?
                                .filter_map(|r| r.ok())
                                .flat_map(|c| {
                                    c.ancestor
                                        .or_else(|| c.our)
                                        .or_else(|| c.their)
                                        .map(|e| e.path)
                                })
                                .collect();
                            for path in paths {
                                conflicts.push(String::from_utf8_lossy(&path).to_string());
                            }
                            repo.cleanup_state().ok();
                            return Ok(PullResult {
                                updated: false,
                                conflicts: Some(conflicts),
                            });
                        }

                        let sig = Signature::now("okf-mcp-server", "agent@okf-mcp-server")
                            .map_err(|e| {
                                StoreError::Other(format!("failed to create signature: {e}"))
                            })?;

                        let tree_oid = index
                            .write_tree()
                            .map_err(|e| StoreError::Other(format!("failed to write tree: {e}")))?;
                        let tree = repo
                            .find_tree(tree_oid)
                            .map_err(|e| StoreError::Other(format!("failed to find tree: {e}")))?;

                        let local_commit_obj =
                            repo.find_commit(local_commit.id()).map_err(|e| {
                                StoreError::Other(format!("failed to find local commit: {e}"))
                            })?;

                        repo.commit(
                            Some("HEAD"),
                            &sig,
                            &sig,
                            &format!("Merge remote-tracking branch '{remote_name}/{branch_name}'"),
                            &tree,
                            &[&local_commit_obj, &fetch_commit],
                        )
                        .map_err(|e| StoreError::Other(format!("merge commit failed: {e}")))?;

                        repo.cleanup_state().ok();
                        return Ok(PullResult {
                            updated: true,
                            conflicts: None,
                        });
                    }
                    Err(e) => {
                        repo.cleanup_state().ok();
                        if e.message().contains("conflict") {
                            let conflicts: Vec<String> = repo
                                .index()
                                .ok()
                                .and_then(|mut idx| {
                                    let iter = idx.conflicts().ok()?;
                                    let mut paths = Vec::new();
                                    for r in iter {
                                        if let Ok(c) = r {
                                            if let Some(e) =
                                                c.ancestor.or_else(|| c.our).or_else(|| c.their)
                                            {
                                                paths.push(
                                                    String::from_utf8_lossy(&e.path).to_string(),
                                                );
                                            }
                                        }
                                    }
                                    Some(paths)
                                })
                                .unwrap_or_default();
                            return Ok(PullResult {
                                updated: false,
                                conflicts: Some(conflicts),
                            });
                        }
                        return Err(StoreError::Other(format!("merge failed: {e}")));
                    }
                }
            }
        } else {
            let fetch_tree = fetch_commit
                .tree()
                .map_err(|e| StoreError::Other(format!("failed to get fetch tree: {e}")))?;

            let mut checkout = git2::build::CheckoutBuilder::new();
            checkout.force();
            repo.checkout_tree(fetch_tree.as_object(), Some(&mut checkout))
                .map_err(|e| StoreError::Other(format!("checkout failed: {e}")))?;

            repo.reference(&branch_ref_name, fetch_commit.id(), true, "pull")
                .map_err(|e| StoreError::Other(format!("failed to create branch ref: {e}")))?;
            repo.set_head(&branch_ref_name)
                .map_err(|e| StoreError::Other(format!("failed to set head: {e}")))?;
        }

        Ok(PullResult {
            updated: true,
            conflicts: None,
        })
    }

    fn create_branch(&self, name: &str, from: Option<&str>) -> StoreResult<String> {
        let repo = self.repo.lock().unwrap();
        let commit = if let Some(source) = from {
            let source_ref = if source.starts_with("refs/") {
                source.to_string()
            } else {
                format!("refs/heads/{source}")
            };
            repo.find_reference(&source_ref)
                .map_err(|e| StoreError::Other(format!("source ref not found '{source}': {e}")))?
                .peel_to_commit()
                .map_err(|e| StoreError::Other(format!("failed to peel source: {e}")))?
        } else {
            repo.head()
                .map_err(|e| StoreError::Other(format!("no HEAD: {e}")))?
                .peel_to_commit()
                .map_err(|e| StoreError::Other(format!("failed to peel HEAD: {e}")))?
        };

        repo.branch(name, &commit, false)
            .map_err(|e| StoreError::Other(format!("failed to create branch '{name}': {e}")))?;

        let branch_ref = format!("refs/heads/{name}");
        let tree = commit
            .tree()
            .map_err(|e| StoreError::Other(format!("failed to get commit tree: {e}")))?;

        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.force();
        repo.checkout_tree(tree.as_object(), Some(&mut checkout))
            .map_err(|e| StoreError::Other(format!("checkout failed: {e}")))?;

        repo.set_head(&branch_ref)
            .map_err(|e| StoreError::Other(format!("failed to set head: {e}")))?;

        Ok(name.to_string())
    }

    fn current_branch(&self) -> StoreResult<String> {
        let repo = self.repo.lock().unwrap();
        repo.head()
            .ok()
            .and_then(|h| h.shorthand().map(String::from))
            .ok_or_else(|| StoreError::Other("no current branch (detached HEAD)".to_string()))
    }

    fn add(&self, path: &str) -> StoreResult<()> {
        let rel = path.trim_start_matches('/');
        let repo = self.repo.lock().unwrap();
        let mut index = repo
            .index()
            .map_err(|e| StoreError::Other(format!("failed to open index: {e}")))?;
        index
            .add_path(Path::new(rel))
            .map_err(|e| StoreError::Other(format!("failed to stage {rel}: {e}")))?;
        index
            .write()
            .map_err(|e| StoreError::Other(format!("failed to write index: {e}")))?;
        Ok(())
    }

    fn stage_all(&self) -> StoreResult<()> {
        let repo = self.repo.lock().unwrap();
        let mut index = repo
            .index()
            .map_err(|e| StoreError::Other(format!("failed to open index: {e}")))?;
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .map_err(|e| StoreError::Other(format!("failed to stage all: {e}")))?;
        index
            .write()
            .map_err(|e| StoreError::Other(format!("failed to write index: {e}")))?;
        Ok(())
    }
}

fn walk_tree(
    repo: &Repository,
    tree: &git2::Tree,
    prefix: &str,
    files: &mut Vec<String>,
) -> StoreResult<()> {
    for entry in tree.iter() {
        let name = entry.name().unwrap_or("");
        let path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        if let Ok(obj) = entry.to_object(repo) {
            if let Some(subtree) = obj.as_tree() {
                walk_tree(repo, subtree, &path, files)?;
            } else if path.ends_with(".md") {
                files.push(path);
            }
        }
    }
    Ok(())
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
