use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rmcp::{
    model::*, schemars, service::RequestContext, transport, ErrorData, RoleServer, ServerHandler,
    ServiceExt,
};
use serde::Deserialize;

use crate::audit::AuditLog;
use crate::bundle::fs_store::LocalFsStore;
use crate::bundle::git_store::GitStore;
use crate::bundle::repo::BundleRepo;
use crate::bundle::store::{BundleStore, GitControl};
use crate::bundle::types::*;
use crate::config::{BundleBackend, ResolvedBundleConfig};

// ---------------------------------------------------------------------------
// Tool argument structures
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BundleArg {
    pub bundle: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListConceptsArgs {
    pub bundle: String,
    pub prefix: Option<String>,
    pub concept_type: Option<String>,
    pub tag: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadConceptArgs {
    pub bundle: String,
    pub concept_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadIndexArgs {
    pub bundle: String,
    pub path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchArgs {
    pub bundle: String,
    pub query: String,
    pub concept_type: Option<String>,
    pub tag: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BacklinksArgs {
    pub bundle: String,
    pub concept_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GraphArgs {
    pub bundle: String,
    pub prefix: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WriteConceptArgs {
    pub bundle: String,
    pub concept_id: String,
    pub data: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteConceptArgs {
    pub bundle: String,
    pub concept_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct IndexSectionInput {
    pub heading: String,
    pub entries: Vec<IndexEntryInput>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct IndexEntryInput {
    pub title: String,
    pub path: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WriteIndexArgs {
    pub bundle: String,
    pub path: String,
    pub sections: Vec<IndexSectionInput>,
    pub okf_version: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LogEntryInput {
    pub label: Option<String>,
    pub text: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AppendLogArgs {
    pub bundle: String,
    pub path: String,
    pub date: Option<String>,
    pub entries: Vec<LogEntryInput>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddCitationArgs {
    pub bundle: String,
    pub concept_id: String,
    pub title: String,
    pub target: String,
}

// Git tool argument structs

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GitStatusArgs {
    pub bundle: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GitDiffArgs {
    pub bundle: String,
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GitCommitArgs {
    pub bundle: String,
    pub message: String,
    pub author: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GitPushArgs {
    pub bundle: String,
    pub remote: Option<String>,
    pub branch: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GitPullArgs {
    pub bundle: String,
    pub remote: Option<String>,
    pub branch: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GitCreateBranchArgs {
    pub bundle: String,
    pub name: String,
    #[serde(rename = "from")]
    pub from: Option<String>,
}

// ---------------------------------------------------------------------------
// MCP Server
// ---------------------------------------------------------------------------

pub struct OkfServer {
    read_tools: Arc<crate::tools::read::ReadTools>,
    write_tools: Arc<crate::tools::write::WriteTools>,
    tools: Vec<Tool>,
    git_controls: HashMap<String, Arc<dyn GitControl>>,
    bundle_configs: HashMap<String, ResolvedBundleConfig>,
    session_branches: Arc<Mutex<HashMap<String, String>>>,
    bundle_repos: HashMap<String, Arc<BundleRepo>>,
}

impl Clone for OkfServer {
    fn clone(&self) -> Self {
        Self {
            read_tools: self.read_tools.clone(),
            write_tools: self.write_tools.clone(),
            tools: self.tools.clone(),
            git_controls: self.git_controls.clone(),
            bundle_configs: self.bundle_configs.clone(),
            session_branches: self.session_branches.clone(),
            bundle_repos: self.bundle_repos.clone(),
        }
    }
}

impl OkfServer {
    pub fn new(
        bundles: Vec<ResolvedBundleConfig>,
        audit_dir: Option<&str>,
        search_index_dir: Option<&str>,
    ) -> Result<Self, String> {
        let mut bundle_repos: HashMap<String, Arc<BundleRepo>> = HashMap::new();
        let mut git_controls: HashMap<String, Arc<dyn GitControl>> = HashMap::new();
        let mut bundle_configs: HashMap<String, ResolvedBundleConfig> = HashMap::new();
        let mut first_audit: Option<Arc<AuditLog>> = None;

        for config in &bundles {
            let name = config.name.clone();
            let root = if config.path.is_absolute() {
                config.path.clone()
            } else {
                std::env::current_dir()
                    .map_err(|e| format!("failed to get current dir: {e}"))?
                    .join(&config.path)
            };

            if !root.exists() {
                std::fs::create_dir_all(&root)
                    .map_err(|e| format!("failed to create bundle directory {root:?}: {e}"))?;
            }

            let (store, git): (Arc<dyn BundleStore>, Option<Arc<dyn GitControl>>) =
                match config.backend {
                    BundleBackend::Fs => (Arc::new(LocalFsStore::new(root.clone())), None),
                    BundleBackend::Git => {
                        let ssh_key = config.auth.as_ref().and_then(|a| a.ssh_key.clone());
                        let token_env = config.auth.as_ref().and_then(|a| a.token_env.clone());
                        let gs = Arc::new(
                            GitStore::new(root.clone(), ssh_key, token_env)
                                .map_err(|e| format!("failed to create GitStore: {e}"))?,
                        );
                        let git_control: Arc<dyn GitControl> = gs.clone();
                        (gs as Arc<dyn BundleStore>, Some(git_control))
                    }
                };

            let repo = Arc::new(BundleRepo::new(
                name.clone(),
                store,
                root,
                search_index_dir.map(std::path::Path::new),
            ));
            bundle_repos.insert(name.clone(), repo);
            bundle_configs.insert(name.clone(), config.clone());

            if let Some(gc) = git {
                git_controls.insert(name.clone(), gc);
            }

            if first_audit.is_none() {
                if let Some(dir) = audit_dir {
                    match AuditLog::new(dir) {
                        Ok(log) => first_audit = Some(Arc::new(log)),
                        Err(e) => eprintln!("warning: failed to create audit log: {e}"),
                    }
                }
            }
        }

        let audit = first_audit;
        let tools = Self::build_tool_list();

        let allowlists: HashMap<String, Option<Vec<String>>> = bundles
            .iter()
            .map(|c| (c.name.clone(), c.write_allowlist.clone()))
            .collect();

        let backends: HashMap<String, crate::config::BundleBackend> = bundles
            .iter()
            .map(|c| (c.name.clone(), c.backend.clone()))
            .collect();

        let read_tools = Arc::new(crate::tools::read::ReadTools::new(
            bundle_repos.clone(),
            backends,
        ));
        let write_tools = Arc::new(crate::tools::write::WriteTools::new(
            bundle_repos.clone(),
            audit,
            allowlists,
        ));

        Ok(Self {
            read_tools,
            write_tools,
            tools,
            git_controls,
            bundle_configs,
            session_branches: Arc::new(Mutex::new(HashMap::new())),
            bundle_repos,
        })
    }

    pub fn start_file_watcher(&self) -> Result<(), String> {
        let watcher = crate::watch::FileWatcher::new(self.bundle_repos.clone());
        watcher.start()
    }

    pub async fn start(self) -> Result<(), Box<dyn std::error::Error>> {
        self.serve(transport::stdio()).await?.waiting().await?;
        Ok(())
    }

    fn tool_schema<T: schemars::JsonSchema>() -> serde_json::Map<String, serde_json::Value> {
        let schema = schemars::schema_for!(T);
        serde_json::to_value(&schema)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    }

    fn build_tool_list() -> Vec<Tool> {
        vec![
            make_tool(
                "okf_list_bundles",
                "List all registered bundles",
                Self::tool_schema::<BundleArg>(),
            ),
            make_tool(
                "okf_list_concepts",
                "List concepts in a bundle. Filter by prefix (path prefix like 'tables/'), concept_type (e.g. Table, View, Metric), or tag.",
                Self::tool_schema::<ListConceptsArgs>(),
            ),
            make_tool(
                "okf_read_concept",
                "Read a concept by its ID",
                Self::tool_schema::<ReadConceptArgs>(),
            ),
            make_tool(
                "okf_read_index",
                "Read or synthesize an index.md for a directory",
                Self::tool_schema::<ReadIndexArgs>(),
            ),
            make_tool(
                "okf_search",
                "Search concepts by query text. Optionally filter by concept_type (Table, View, etc.) or tag.",
                Self::tool_schema::<SearchArgs>(),
            ),
            make_tool(
                "okf_get_backlinks",
                "Get concepts that link to a given concept",
                Self::tool_schema::<BacklinksArgs>(),
            ),
            make_tool(
                "okf_get_graph",
                "Get the full link graph for a bundle or subdirectory",
                Self::tool_schema::<GraphArgs>(),
            ),
            make_tool(
                "okf_validate_bundle",
                "Validate a bundle against OKF conformance rules",
                Self::tool_schema::<BundleArg>(),
            ),
            make_tool(
                "okf_write_concept",
                "Write a concept to a bundle. The 'data' param is a JSON string with fields: type (required), title, description, resource, tags (array or comma-separated string), timestamp, body (markdown), body_sections (array of {heading, content}), body_section_mode (replace|merge), mode (create|update|upsert, default upsert).",
                Self::tool_schema::<WriteConceptArgs>(),
            ),
            make_tool(
                "okf_delete_concept",
                "Delete a concept from a bundle",
                Self::tool_schema::<DeleteConceptArgs>(),
            ),
            make_tool(
                "okf_write_index",
                "Write or update an index.md for a directory",
                Self::tool_schema::<WriteIndexArgs>(),
            ),
            make_tool(
                "okf_append_log",
                "Append entries to a log.md file",
                Self::tool_schema::<AppendLogArgs>(),
            ),
            make_tool(
                "okf_add_citation",
                "Add a citation to a concept's # Citations section",
                Self::tool_schema::<AddCitationArgs>(),
            ),
            // Git tools
            make_tool(
                "okf_git_status",
                "Show git status (staged, unstaged, untracked files)",
                Self::tool_schema::<GitStatusArgs>(),
            ),
            make_tool(
                "okf_git_diff",
                "Show git diff for staged/unstaged changes",
                Self::tool_schema::<GitDiffArgs>(),
            ),
            make_tool(
                "okf_git_commit",
                "Commit currently staged changes",
                Self::tool_schema::<GitCommitArgs>(),
            ),
            make_tool(
                "okf_git_push",
                "Push to a remote repository",
                Self::tool_schema::<GitPushArgs>(),
            ),
            make_tool(
                "okf_git_pull",
                "Pull and merge from a remote repository",
                Self::tool_schema::<GitPullArgs>(),
            ),
            make_tool(
                "okf_git_create_branch",
                "Create and switch to a new branch",
                Self::tool_schema::<GitCreateBranchArgs>(),
            ),
        ]
    }

    fn text(data: impl serde::Serialize) -> Result<CallToolResult, ErrorData> {
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&data).unwrap_or_default(),
        )]))
    }

    fn err(msg: impl Into<String>) -> ErrorData {
        ErrorData::internal_error(msg.into(), None::<serde_json::Value>)
    }

    fn parse_args<T: serde::de::DeserializeOwned>(
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<T, ErrorData> {
        serde_json::from_value(serde_json::Value::Object(args.unwrap_or_default()))
            .map_err(|e| ErrorData::invalid_params(e.to_string(), None::<serde_json::Value>))
    }

    fn get_git_control(&self, bundle: &str) -> Result<&Arc<dyn GitControl>, ErrorData> {
        self.git_controls.get(bundle).ok_or_else(|| {
            ErrorData::invalid_params(
                format!("bundle '{bundle}' is not git-backed or not found"),
                None::<serde_json::Value>,
            )
        })
    }

    // Ensure a session branch exists for the bundle if branch_policy is session-branch.
    // Called before any write operation on a git-backed bundle.
    fn ensure_session_branch(&self, bundle: &str) -> Result<(), ErrorData> {
        let config = self.bundle_configs.get(bundle).ok_or_else(|| {
            ErrorData::invalid_params(
                format!("bundle not found: {bundle}"),
                None::<serde_json::Value>,
            )
        })?;

        if config.backend != BundleBackend::Git {
            return Ok(());
        }

        let policy = config.branch_policy.as_deref().unwrap_or("direct");
        if policy != "session-branch" {
            return Ok(());
        }

        let mut sessions = self.session_branches.lock().map_err(|e| {
            Self::err(format!("session lock poisoned: {e}"))
        })?;
        if sessions.contains_key(bundle) {
            return Ok(());
        }

        let git = self.get_git_control(bundle)?;
        let current_branch = git.current_branch().map_err(|e| Self::err(e.to_string()))?;
        let default_branch = config.default_branch.as_deref().unwrap_or("main");

        if current_branch != default_branch {
            return Ok(());
        }

        let ts = chrono::Utc::now().format("%Y%m%d%H%M%S");
        let session_branch = format!("okf/agent-session-{ts}");

        git.create_branch(&session_branch, Some(current_branch.as_str()))
            .map_err(|e| Self::err(e.to_string()))?;

        sessions.insert(bundle.to_string(), session_branch.clone());
        tracing::info!("Created session branch {session_branch} for bundle {bundle}");

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ServerHandler — MCP protocol implementation
// ---------------------------------------------------------------------------

impl ServerHandler for OkfServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                "OKF MCP Server — read, search, and write OKF knowledge bundles.".to_string(),
            ),
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools.clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let name = &*request.name;
        let args = request.arguments;

        match name {
            // Read tools
            "okf_list_bundles" => self.call_list_bundles(),
            "okf_list_concepts" => self.call_list_concepts(args),
            "okf_read_concept" => self.call_read_concept(args),
            "okf_read_index" => self.call_read_index(args),
            "okf_search" => self.call_search(args),
            "okf_get_backlinks" => self.call_get_backlinks(args),
            "okf_get_graph" => self.call_get_graph(args),
            "okf_validate_bundle" => self.call_validate_bundle(args),
            // Write tools
            "okf_write_concept" => self.call_write_concept(args),
            "okf_delete_concept" => self.call_delete_concept(args),
            "okf_write_index" => self.call_write_index(args),
            "okf_append_log" => self.call_append_log(args),
            "okf_add_citation" => self.call_add_citation(args),
            // Git tools
            "okf_git_status" => self.call_git_status(args),
            "okf_git_diff" => self.call_git_diff(args),
            "okf_git_commit" => self.call_git_commit(args),
            "okf_git_push" => self.call_git_push(args),
            "okf_git_pull" => self.call_git_pull(args),
            "okf_git_create_branch" => self.call_git_create_branch(args),
            _ => Err(ErrorData::new(
                ErrorCode::METHOD_NOT_FOUND,
                format!("unknown tool: {name}"),
                None::<serde_json::Value>,
            )),
        }
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult {
            resources: vec![
                Annotated::new(
                    RawResource {
                        uri: "okf://{bundle}/{concept_id}".to_string(),
                        name: "OKF Concept".to_string(),
                        title: None,
                        description: Some("Read a concept by its ID".to_string()),
                        mime_type: Some("text/markdown".to_string()),
                        size: None,
                        icons: None,
                        meta: None,
                    },
                    None,
                ),
                Annotated::new(
                    RawResource {
                        uri: "okf://{bundle}/_index/{path}".to_string(),
                        name: "OKF Index".to_string(),
                        title: None,
                        description: Some("Read a directory index".to_string()),
                        mime_type: Some("text/markdown".to_string()),
                        size: None,
                        icons: None,
                        meta: None,
                    },
                    None,
                ),
            ],
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        let uri = request.uri.as_str().to_string();
        let uri = uri.strip_prefix("okf://").ok_or_else(|| {
            ErrorData::invalid_params("invalid URI scheme", None::<serde_json::Value>)
        })?;

        if uri.contains('{') {
            return Ok(ReadResourceResult {
                contents: vec![ResourceContents::text(
                    "Use a concrete URI: okf://{bundle}/{concept_id} or okf://{bundle}/_index/{path}",
                    request.uri.as_str(),
                )],
            });
        }

        let slash_pos = uri.find('/').ok_or_else(|| {
            ErrorData::invalid_params(format!("invalid URI: {uri}"), None::<serde_json::Value>)
        })?;
        let bundle_name = &uri[..slash_pos];
        let remaining = &uri[slash_pos + 1..];

        if remaining.starts_with("_index/") {
            let dir_path = &remaining["_index/".len()..];
            let result = self
                .read_tools
                .read_index(bundle_name, dir_path)
                .map_err(|e| ErrorData::internal_error(e, None::<serde_json::Value>))?;
            Ok(ReadResourceResult {
                contents: vec![ResourceContents::text(
                    result.rendered,
                    request.uri.as_str(),
                )],
            })
        } else {
            let concept = self
                .read_tools
                .read_concept(bundle_name, remaining)
                .map_err(|e| ErrorData::invalid_params(e, None::<serde_json::Value>))?;
            let text = format!(
                "---\ntype: {}\n{}\n---\n\n{}",
                concept.frontmatter.r#type,
                render_fm(&concept.frontmatter),
                concept.body
            );
            Ok(ReadResourceResult {
                contents: vec![ResourceContents::text(text, request.uri.as_str())],
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Tool handler methods — reads
// ---------------------------------------------------------------------------

impl OkfServer {
    fn call_list_bundles(&self) -> Result<CallToolResult, ErrorData> {
        let mut info: Vec<serde_json::Value> = Vec::new();
        for (name, config) in &self.bundle_configs {
            info.push(serde_json::json!({
                "name": name,
                "backend": if config.backend == BundleBackend::Git { "git" } else { "fs" },
                "path": config.path.to_string_lossy(),
                "default_branch": config.default_branch,
            }));
        }
        Self::text(&info)
    }

    fn call_list_concepts(
        &self,
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let a: ListConceptsArgs = Self::parse_args(args)?;
        self.read_tools
            .list_concepts(
                &a.bundle,
                a.prefix.as_deref(),
                a.concept_type.as_deref(),
                a.tag.as_deref(),
            )
            .map_err(Self::err)
            .and_then(Self::text)
    }

    fn call_read_concept(
        &self,
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let a: ReadConceptArgs = Self::parse_args(args)?;
        self.read_tools
            .read_concept(&a.bundle, &a.concept_id)
            .map_err(Self::err)
            .and_then(Self::text)
    }

    fn call_read_index(
        &self,
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let a: ReadIndexArgs = Self::parse_args(args)?;
        self.read_tools
            .read_index(&a.bundle, &a.path)
            .map_err(Self::err)
            .and_then(Self::text)
    }

    fn call_search(
        &self,
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let a: SearchArgs = Self::parse_args(args)?;
        self.read_tools
            .search(&a.bundle, &a.query, a.concept_type.as_deref(), a.tag.as_deref())
            .map_err(Self::err)
            .and_then(Self::text)
    }

    fn call_get_backlinks(
        &self,
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let a: BacklinksArgs = Self::parse_args(args)?;
        self.read_tools
            .get_backlinks(&a.bundle, &a.concept_id)
            .map_err(Self::err)
            .and_then(Self::text)
    }

    fn call_get_graph(
        &self,
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let a: GraphArgs = Self::parse_args(args)?;
        self.read_tools
            .get_graph(&a.bundle, a.prefix.as_deref())
            .map_err(Self::err)
            .and_then(Self::text)
    }

    fn call_validate_bundle(
        &self,
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let a: BundleArg = Self::parse_args(args)?;
        self.read_tools
            .validate_bundle(&a.bundle)
            .map_err(Self::err)
            .and_then(Self::text)
    }
}

// ---------------------------------------------------------------------------
// Tool handler methods — writes
// ---------------------------------------------------------------------------

impl OkfServer {
    fn call_write_concept(
        &self,
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let a: WriteConceptArgs = Self::parse_args(args)?;
        self.ensure_session_branch(&a.bundle)?;

        let data: serde_json::Value = serde_json::from_str(&a.data)
            .map_err(|e| ErrorData::invalid_params(format!("invalid data JSON: {e}"), None::<serde_json::Value>))?;

        if !data.is_object() {
            return Err(ErrorData::invalid_params("data must be a JSON object", None::<serde_json::Value>));
        }

        let r#type = data.get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ErrorData::invalid_params("data.type is required and must be a string", None::<serde_json::Value>))?;
        if r#type.is_empty() {
            return Err(ErrorData::invalid_params("data.type must not be empty", None::<serde_json::Value>));
        }

        if let Some(t) = data.get("title") {
            if !t.is_string() {
                return Err(ErrorData::invalid_params("data.title must be a string", None::<serde_json::Value>));
            }
        }
        if let Some(d) = data.get("description") {
            if !d.is_string() {
                return Err(ErrorData::invalid_params("data.description must be a string", None::<serde_json::Value>));
            }
        }
        if let Some(r) = data.get("resource") {
            if !r.is_string() {
                return Err(ErrorData::invalid_params("data.resource must be a string", None::<serde_json::Value>));
            }
        }
        if let Some(ts) = data.get("timestamp") {
            if !ts.is_string() {
                return Err(ErrorData::invalid_params("data.timestamp must be a string", None::<serde_json::Value>));
            }
        }
        if let Some(tags_val) = data.get("tags") {
            match tags_val {
                serde_json::Value::Array(arr) => {
                    for t in arr {
                        if !t.is_string() {
                            return Err(ErrorData::invalid_params("data.tags array elements must be strings", None::<serde_json::Value>));
                        }
                    }
                }
                serde_json::Value::String(_) => {}
                _ => {
                    return Err(ErrorData::invalid_params("data.tags must be an array of strings or a comma-separated string", None::<serde_json::Value>));
                }
            }
        }

        let mode = data.get("mode").and_then(|v| v.as_str()).unwrap_or("upsert");
        if !matches!(mode, "create" | "update" | "upsert") {
            return Err(ErrorData::invalid_params(
                format!("data.mode must be 'create', 'update', or 'upsert', got '{mode}'"),
                None::<serde_json::Value>,
            ));
        }

        if let Some(bsm) = data.get("body_section_mode") {
            let v = bsm.as_str().ok_or_else(|| {
                ErrorData::invalid_params("data.body_section_mode must be a string", None::<serde_json::Value>)
            })?;
            if !matches!(v, "replace" | "merge") {
                return Err(ErrorData::invalid_params(
                    format!("data.body_section_mode must be 'replace' or 'merge', got '{v}'"),
                    None::<serde_json::Value>,
                ));
            }
        }

        if let Some(sections) = data.get("body_sections") {
            let arr = sections.as_array().ok_or_else(|| {
                ErrorData::invalid_params("data.body_sections must be an array", None::<serde_json::Value>)
            })?;
            for (i, s) in arr.iter().enumerate() {
                let obj = s.as_object().ok_or_else(|| {
                    ErrorData::invalid_params(format!("data.body_sections[{i}] must be an object"), None::<serde_json::Value>)
                })?;
                if !obj.contains_key("heading") || !obj.get("heading").and_then(|v| v.as_str()).is_some() {
                    return Err(ErrorData::invalid_params(
                        format!("data.body_sections[{i}].heading is required and must be a string"),
                        None::<serde_json::Value>,
                    ));
                }
                if !obj.contains_key("content") || !obj.get("content").and_then(|v| v.as_str()).is_some() {
                    return Err(ErrorData::invalid_params(
                        format!("data.body_sections[{i}].content is required and must be a string"),
                        None::<serde_json::Value>,
                    ));
                }
            }
        }

        let body = data.get("body").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let body_section_mode = data.get("body_section_mode").and_then(|v| v.as_str()).map(String::from);
        let body_sections = data.get("body_sections").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|s| {
                        let heading = s.get("heading")?.as_str()?.to_string();
                        let content = s.get("content")?.as_str()?.to_string();
                        Some(BodySection { heading, content })
                    })
                    .collect::<Vec<_>>()
            })
        });

        let fm = Frontmatter {
            r#type: r#type.to_string(),
            title: data.get("title").and_then(|v| v.as_str()).map(String::from),
            description: data.get("description").and_then(|v| v.as_str()).map(String::from),
            resource: data.get("resource").and_then(|v| v.as_str()).map(String::from),
            tags: data.get("tags").and_then(|v| {
                v.as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|t| t.as_str().map(String::from))
                        .collect::<Vec<_>>()
                })
                .or_else(|| v.as_str().map(|s| {
                    s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()
                }))
            }),
            timestamp: data.get("timestamp").and_then(|v| v.as_str()).map(String::from),
            extra: serde_yaml::Mapping::new(),
        };
        self.write_tools
            .write_concept(&a.bundle, &a.concept_id, fm, body, body_sections, body_section_mode, &mode)
            .map_err(Self::err)
            .and_then(Self::text)
    }

    fn call_delete_concept(
        &self,
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let a: DeleteConceptArgs = Self::parse_args(args)?;
        self.ensure_session_branch(&a.bundle)?;
        self.write_tools
            .delete_concept(&a.bundle, &a.concept_id)
            .map_err(Self::err)
            .and_then(|deleted| Self::text(serde_json::json!({ "deleted": deleted })))
    }

    fn call_write_index(
        &self,
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let a: WriteIndexArgs = Self::parse_args(args)?;
        self.ensure_session_branch(&a.bundle)?;
        let sections: Vec<IndexSection> = a
            .sections
            .into_iter()
            .map(|s| IndexSection {
                heading: s.heading,
                entries: s
                    .entries
                    .into_iter()
                    .map(|e| IndexEntry {
                        title: e.title,
                        path: e.path,
                        description: e.description,
                    })
                    .collect(),
            })
            .collect();
        self.write_tools
            .write_index(&a.bundle, &a.path, sections, a.okf_version)
            .map_err(Self::err)
            .and_then(|rendered| Self::text(serde_json::json!({ "rendered": rendered })))
    }

    fn call_append_log(
        &self,
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let a: AppendLogArgs = Self::parse_args(args)?;
        self.ensure_session_branch(&a.bundle)?;
        let date = a
            .date
            .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
        let entries: Vec<LogEntry> = a
            .entries
            .into_iter()
            .map(|e| LogEntry {
                label: e.label,
                text: e.text,
            })
            .collect();
        self.write_tools
            .append_log(&a.bundle, &a.path, &date, entries)
            .map_err(Self::err)
            .and_then(|updated| Self::text(serde_json::json!({ "updated": updated })))
    }

    fn call_add_citation(
        &self,
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let a: AddCitationArgs = Self::parse_args(args)?;
        self.ensure_session_branch(&a.bundle)?;
        self.write_tools
            .add_citation(&a.bundle, &a.concept_id, &a.title, &a.target)
            .map_err(Self::err)
            .and_then(|updated| Self::text(serde_json::json!({ "updated": updated })))
    }
}

// ---------------------------------------------------------------------------
// Tool handler methods — git operations
// ---------------------------------------------------------------------------

impl OkfServer {
    fn call_git_status(
        &self,
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let a: GitStatusArgs = Self::parse_args(args)?;
        let git = self.get_git_control(&a.bundle)?.clone();
        git.status()
            .map_err(|e| Self::err(e.to_string()))
            .and_then(Self::text)
    }

    fn call_git_diff(
        &self,
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let a: GitDiffArgs = Self::parse_args(args)?;
        let git = self.get_git_control(&a.bundle)?.clone();
        git.diff(a.path.as_deref())
            .map_err(|e| Self::err(e.to_string()))
            .and_then(Self::text)
    }

    fn call_git_commit(
        &self,
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let a: GitCommitArgs = Self::parse_args(args)?;
        let git = self.get_git_control(&a.bundle)?.clone();
        git.commit(&a.message, a.author.as_deref())
            .map_err(|e| Self::err(e.to_string()))
            .and_then(|sha| Self::text(serde_json::json!({ "sha": sha })))
    }

    fn call_git_push(
        &self,
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let a: GitPushArgs = Self::parse_args(args)?;
        let git = self.get_git_control(&a.bundle)?.clone();
        git.push(a.remote.as_deref(), a.branch.as_deref())
            .map_err(|e| Self::err(e.to_string()))
            .and_then(Self::text)
    }

    fn call_git_pull(
        &self,
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let a: GitPullArgs = Self::parse_args(args)?;
        let git = self.get_git_control(&a.bundle)?.clone();
        git.pull(a.remote.as_deref(), a.branch.as_deref())
            .map_err(|e| Self::err(e.to_string()))
            .and_then(Self::text)
    }

    fn call_git_create_branch(
        &self,
        args: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<CallToolResult, ErrorData> {
        let a: GitCreateBranchArgs = Self::parse_args(args)?;
        let git = self.get_git_control(&a.bundle)?.clone();
        git.create_branch(&a.name, a.from.as_deref())
            .map_err(|e| Self::err(e.to_string()))
            .and_then(|name| Self::text(serde_json::json!({ "branch": name })))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_tool(
    name: &'static str,
    description: &'static str,
    input_schema: serde_json::Map<String, serde_json::Value>,
) -> Tool {
    Tool {
        name: name.into(),
        title: None,
        description: Some(description.into()),
        input_schema: Arc::new(input_schema),
        output_schema: None,
        annotations: None,
        execution: None,
        icons: None,
        meta: None,
    }
}

fn render_fm(fm: &Frontmatter) -> String {
    let mut parts = Vec::new();
    if let Some(ref title) = fm.title {
        parts.push(format!("title: {title}"));
    }
    if let Some(ref desc) = fm.description {
        parts.push(format!("description: {desc}"));
    }
    if let Some(ref resource) = fm.resource {
        parts.push(format!("resource: {resource}"));
    }
    if let Some(ref tags) = fm.tags {
        parts.push(format!("tags: [{}]", tags.join(", ")));
    }
    if let Some(ref ts) = fm.timestamp {
        parts.push(format!("timestamp: {ts}"));
    }
    parts.join("\n")
}
