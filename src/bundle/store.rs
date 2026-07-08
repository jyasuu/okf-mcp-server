pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("path safety error: {0}")]
    PathSafety(#[from] crate::bundle::path_safety::PathSafetyError),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error("{0}")]
    Other(String),
}

/// Raw byte-level file operations for one bundle.
pub trait BundleStore: Send + Sync {
    /// List all files under the bundle root, optionally filtered by prefix.
    fn list_files(&self, prefix: Option<&str>) -> StoreResult<Vec<String>>;

    /// Read the raw content of a file at the given relative path.
    fn read_raw(&self, path: &str) -> StoreResult<String>;

    /// Write content to a file atomically (temp + rename).
    fn write_raw(&self, path: &str, content: &str) -> StoreResult<()>;

    /// Delete a file at the given relative path.
    fn delete_raw(&self, path: &str) -> StoreResult<()>;

    /// Check if a file exists at the given relative path.
    fn exists(&self, path: &str) -> bool;
}

/// Git-specific operations for git-backed bundles.
pub trait GitControl: Send + Sync {
    fn status(&self) -> StoreResult<GitStatus>;
    fn diff(&self, path: Option<&str>) -> StoreResult<String>;
    fn commit(&self, message: &str, author: Option<&str>) -> StoreResult<String>;
    fn push(&self, remote: Option<&str>, branch: Option<&str>) -> StoreResult<PushResult>;
    fn pull(&self, remote: Option<&str>, branch: Option<&str>) -> StoreResult<PullResult>;
    fn create_branch(&self, name: &str, from: Option<&str>) -> StoreResult<String>;
    fn current_branch(&self) -> StoreResult<String>;
    fn add(&self, path: &str) -> StoreResult<()>;
    fn stage_all(&self) -> StoreResult<()>;
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GitStatus {
    pub staged: Vec<String>,
    pub unstaged: Vec<String>,
    pub untracked: Vec<String>,
    pub branch: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PushResult {
    pub pushed_branch: String,
    pub remote: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PullResult {
    pub updated: bool,
    pub conflicts: Option<Vec<String>>,
}
