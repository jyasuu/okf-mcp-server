use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::bundle::search_index::SearchIndex;
use crate::bundle::store::{BundleStore, StoreError, StoreResult};
use crate::bundle::types::*;

pub struct BundleRepo {
    name: String,
    store: Arc<dyn BundleStore>,
    root: PathBuf,
    write_mutex: Mutex<()>,
    search_index: Option<SearchIndex>,
}

impl BundleRepo {
    pub fn new(
        name: String,
        store: Arc<dyn BundleStore>,
        root: PathBuf,
        search_index_dir: Option<&Path>,
    ) -> Self {
        let search_index = match search_index_dir {
            Some(dir) => {
                let index_dir = dir.join(&name);
                match SearchIndex::new(&index_dir) {
                    Ok(idx) => {
                        tracing::info!("search index created at {:?}", index_dir);
                        Some(idx)
                    }
                    Err(e) => {
                        tracing::warn!("failed to create search index: {e}");
                        None
                    }
                }
            }
            None => None,
        };

        Self {
            name,
            store,
            root,
            write_mutex: Mutex::new(()),
            search_index,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn store(&self) -> &dyn BundleStore {
        self.store.as_ref()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    // --- Listing ---

    pub fn list_concepts(
        &self,
        prefix: Option<&str>,
        type_filter: Option<&str>,
        tag_filter: Option<&str>,
    ) -> StoreResult<Vec<ConceptId>> {
        let prefix = prefix
            .map(|p| {
                let clean = p.trim_start_matches('/').trim_end_matches('/');
                if clean.is_empty() {
                    None
                } else {
                    Some(clean.to_string())
                }
            })
            .flatten();

        let all_files = self.store.list_files(prefix.as_deref())?;
        let mut concepts = Vec::new();

        for file in &all_files {
            if file.ends_with("index.md") || file.ends_with("log.md") {
                continue;
            }

            if let Some(t) = type_filter {
                match self
                    .read_concept_frontmatter_only(&ConceptId::new(file.trim_end_matches(".md")))
                {
                    Ok(fm) => {
                        if fm.r#type != t {
                            continue;
                        }
                    }
                    Err(_) => continue,
                }
            }

            if let Some(tag) = tag_filter {
                match self
                    .read_concept_frontmatter_only(&ConceptId::new(file.trim_end_matches(".md")))
                {
                    Ok(fm) => {
                        let has_tag = fm
                            .tags
                            .as_ref()
                            .map_or(false, |tgs| tgs.iter().any(|t| t == tag));
                        if !has_tag {
                            continue;
                        }
                    }
                    Err(_) => continue,
                }
            }

            concepts.push(ConceptId::new(file.trim_end_matches(".md")));
        }

        Ok(concepts)
    }

    // --- Reading ---

    pub fn read_concept(&self, id: &ConceptId) -> StoreResult<Concept> {
        let path = id.to_path();
        let content = self.store.read_raw(&path)?;
        let (frontmatter, body) = parse_frontmatter(&content)
            .map_err(|e| StoreError::Other(format!("failed to parse frontmatter: {e}")))?;
        Ok(Concept {
            id: id.clone(),
            frontmatter,
            body,
        })
    }

    pub fn read_concept_frontmatter_only(&self, id: &ConceptId) -> StoreResult<Frontmatter> {
        let path = id.to_path();
        let content = self.store.read_raw(&path)?;
        let (frontmatter, _) = parse_frontmatter(&content)
            .map_err(|e| StoreError::Other(format!("failed to parse frontmatter: {e}")))?;
        Ok(frontmatter)
    }

    pub fn read_raw(&self, path: &str) -> StoreResult<String> {
        self.store.read_raw(path)
    }

    // --- Writing ---

    pub fn write_concept(&self, concept: Concept, mode: WriteMode) -> StoreResult<Concept> {
        let _guard = self
            .write_mutex
            .lock()
            .map_err(|e| StoreError::Other(format!("lock poisoned: {e}")))?;

        let path = concept.id.to_path();
        let exists = self.store.exists(&path);

        match mode {
            WriteMode::Create => {
                if exists {
                    return Err(StoreError::AlreadyExists(concept.id.to_string()));
                }
            }
            WriteMode::Update => {
                if !exists {
                    return Err(StoreError::NotFound(concept.id.to_string()));
                }
            }
            WriteMode::Upsert => { /* always allowed */ }
        }

        // Validate required frontmatter
        if concept.frontmatter.r#type.trim().is_empty() {
            return Err(StoreError::Other(
                "frontmatter type is required and must be non-empty".to_string(),
            ));
        }

        let content = serialize_concept(&concept.frontmatter, &concept.body);
        self.store.write_raw(&path, &content)?;

        // Update search index
        if let Some(ref idx) = self.search_index {
            let _ = idx.remove_concept(concept.id.as_str());
            let _ = idx.add_concept(&concept);
        }

        Ok(concept)
    }

    pub fn delete_concept(&self, id: &ConceptId) -> StoreResult<bool> {
        let _guard = self
            .write_mutex
            .lock()
            .map_err(|e| StoreError::Other(format!("lock poisoned: {e}")))?;
        let path = id.to_path();
        if !self.store.exists(&path) {
            return Err(StoreError::NotFound(id.to_string()));
        }
        self.store.delete_raw(&path)?;

        // Update search index
        if let Some(ref idx) = self.search_index {
            let _ = idx.remove_concept(id.as_str());
        }

        Ok(true)
    }

    // --- Index reading/writing ---

    pub fn read_index(&self, dir_path: &str) -> StoreResult<IndexReadResult> {
        let index_path = if dir_path.is_empty() || dir_path == "/" {
            "index.md".to_string()
        } else {
            let dir = dir_path.trim_start_matches('/').trim_end_matches('/');
            format!("{dir}/index.md")
        };

        if self.store.exists(&index_path) {
            let content = self.store.read_raw(&index_path)?;
            let (fm, body) = match parse_frontmatter(&content) {
                Ok(v) => v,
                Err(_) => {
                    return Ok(IndexReadResult {
                        rendered: content.clone(),
                        okf_version: None,
                        sections: None,
                    });
                }
            };
            let okf_version = if dir_path.is_empty() || dir_path == "/" {
                fm.extra
                    .get("okf_version")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            } else {
                None
            };
            let sections = parse_index_body(&body);
            Ok(IndexReadResult {
                rendered: content,
                okf_version,
                sections,
            })
        } else {
            // Synthesize a listing from child concepts
            let sections = self.synthesize_index(dir_path)?;
            let rendered = render_index(&sections, None);
            Ok(IndexReadResult {
                rendered,
                okf_version: None,
                sections: Some(sections),
            })
        }
    }

    fn synthesize_index(&self, dir_path: &str) -> StoreResult<Vec<IndexSection>> {
        let prefix = if dir_path.is_empty() || dir_path == "/" {
            None
        } else {
            Some(format!(
                "{}/",
                dir_path.trim_start_matches('/').trim_end_matches('/')
            ))
        };

        let files = self.store.list_files(prefix.as_deref())?;
        let mut concepts = Vec::new();
        let mut subdirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

        for file in &files {
            if file.ends_with("index.md") || file.ends_with("log.md") {
                continue;
            }

            let rel = if let Some(ref p) = prefix {
                file.strip_prefix(p).unwrap_or(file).trim_start_matches('/')
            } else {
                file.as_str()
            };

            if let Some(slash_pos) = rel.find('/') {
                let subdir = &rel[..slash_pos];
                let subdir_prefix = prefix
                    .as_ref()
                    .map(|p| format!("{}{}", p, subdir))
                    .unwrap_or_else(|| subdir.to_string());
                if file.trim_end_matches(".md") != subdir_prefix {
                    subdirs.insert(subdir.to_string());
                    continue;
                }
            }

            match self.read_concept_frontmatter_only(&ConceptId::new(file.trim_end_matches(".md")))
            {
                Ok(fm) => {
                    let title = fm
                        .title
                        .clone()
                        .unwrap_or_else(|| file.trim_end_matches(".md").to_string());
                    concepts.push(IndexEntry {
                        title,
                        path: format!("/{file}"),
                        description: fm.description.clone(),
                    });
                }
                Err(_) => {
                    concepts.push(IndexEntry {
                        title: file.trim_end_matches(".md").to_string(),
                        path: format!("/{file}"),
                        description: None,
                    });
                }
            }
        }

        let mut sections = Vec::new();
        if !concepts.is_empty() {
            concepts.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
            sections.push(IndexSection {
                heading: "Concepts".to_string(),
                entries: concepts,
            });
        }

        if !subdirs.is_empty() {
            let entries: Vec<IndexEntry> = subdirs
                .into_iter()
                .map(|d| {
                    let dir_path = prefix
                        .as_ref()
                        .map(|p| format!("{}{}", p, d))
                        .unwrap_or_else(|| d.clone());
                    IndexEntry {
                        title: d.clone(),
                        path: format!("/{dir_path}/"),
                        description: None,
                    }
                })
                .collect();
            sections.push(IndexSection {
                heading: "Subdirectories".to_string(),
                entries,
            });
        }

        Ok(sections)
    }

    pub fn write_index(&self, dir_path: &str, data: IndexData) -> StoreResult<String> {
        let _guard = self
            .write_mutex
            .lock()
            .map_err(|e| StoreError::Other(format!("lock poisoned: {e}")))?;

        let is_root = dir_path.is_empty() || dir_path == "/";
        let index_path = if is_root {
            "index.md".to_string()
        } else {
            let dir = dir_path.trim_start_matches('/').trim_end_matches('/');
            format!("{dir}/index.md")
        };

        if data.okf_version.is_some() && !is_root {
            return Err(StoreError::Other(
                "okf_version can only be set on root index.md".to_string(),
            ));
        }

        let rendered = render_index(&data.sections, data.okf_version);
        self.store.write_raw(&index_path, &rendered)?;
        Ok(rendered)
    }

    // --- Log ---

    pub fn append_log(
        &self,
        dir_path: &str,
        date: &str,
        entries: &[LogEntry],
    ) -> StoreResult<String> {
        let _guard = self
            .write_mutex
            .lock()
            .map_err(|e| StoreError::Other(format!("lock poisoned: {e}")))?;

        let log_path = if dir_path.is_empty() || dir_path == "/" {
            "log.md".to_string()
        } else {
            let dir = dir_path.trim_start_matches('/').trim_end_matches('/');
            format!("{dir}/log.md")
        };

        let existing = if self.store.exists(&log_path) {
            self.store.read_raw(&log_path)?
        } else {
            String::new()
        };

        let updated = append_to_log(&existing, date, entries);
        self.store.write_raw(&log_path, &updated)?;
        Ok(updated)
    }

    // --- Citations ---

    pub fn add_citation(&self, id: &ConceptId, citation: &CitationInput) -> StoreResult<String> {
        let _guard = self
            .write_mutex
            .lock()
            .map_err(|e| StoreError::Other(format!("lock poisoned: {e}")))?;

        let path = id.to_path();
        let content = self.store.read_raw(&path)?;
        let (fm, body) = parse_frontmatter(&content)
            .map_err(|e| StoreError::Other(format!("failed to parse frontmatter: {e}")))?;

        let updated = add_citation_to_body(&body, citation);
        let new_content = serialize_concept(&fm, &updated);
        self.store.write_raw(&path, &new_content)?;
        Ok(updated)
    }

    // --- Validation ---

    pub fn validate(&self) -> StoreResult<ValidationResult> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        let files = self.store.list_files(None)?;
        let mut concepts_seen = std::collections::HashSet::new();

        for file in &files {
            if !file.ends_with(".md") {
                continue;
            }

            let is_reserved = file == "index.md"
                || file.ends_with("/index.md")
                || file == "log.md"
                || file.ends_with("/log.md");

            if is_reserved {
                // Check reserved files follow structure
                match self.store.read_raw(file) {
                    Ok(content) => {
                        if let Ok((fm, _)) = parse_frontmatter(&content) {
                            // Reserved files at non-root level should have no type
                            if file != "index.md" && fm.extra.contains_key("okf_version") {
                                errors.push(format!(
                                    "okf_version present in non-root index.md: {file}"
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        errors.push(format!("could not read reserved file {file}: {e}"));
                    }
                }
                continue;
            }

            // Non-reserved .md file: must have parseable frontmatter with non-empty type
            let content = match self.store.read_raw(file) {
                Ok(c) => c,
                Err(e) => {
                    errors.push(format!("could not read {file}: {e}"));
                    continue;
                }
            };

            let (fm, _) = match parse_frontmatter(&content) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(format!(
                        "missing or unparseable YAML frontmatter in {file}: {e}"
                    ));
                    continue;
                }
            };

            // Hard rule: type is non-empty
            if fm.r#type.trim().is_empty() {
                errors.push(format!("empty type in frontmatter of {file}"));
            }

            // Soft warnings
            for w in fm.warnings() {
                warnings.push(format!("{w} in {file}"));
            }

            concepts_seen.insert(file.clone());
        }

        // Check for missing index.md in directories
        let mut dirs_with_md = std::collections::HashSet::new();
        for file in &files {
            if let Some(parent) = file.rsplit_once('/') {
                dirs_with_md.insert(parent.0.to_string());
            } else {
                dirs_with_md.insert(String::new());
            }
        }

        for dir in &dirs_with_md {
            let index_path = if dir.is_empty() {
                "index.md".to_string()
            } else {
                format!("{dir}/index.md")
            };
            if !self.store.exists(&index_path) {
                let display = if dir.is_empty() {
                    "root (/) / ."
                } else {
                    dir.as_str()
                };
                warnings.push(format!("missing index.md in directory: {display}"));
            }
        }

        Ok(ValidationResult { errors, warnings })
    }

    // --- Graph ---

    pub fn get_backlinks(&self, id: &ConceptId) -> StoreResult<Vec<ConceptId>> {
        let all_concepts = self.list_concepts(None, None, None)?;
        let mut backlinks = Vec::new();

        for candidate in &all_concepts {
            if candidate == id {
                continue;
            }
            match self.read_concept(candidate) {
                Ok(concept) => {
                    let links = extract_links(candidate, &concept.body);
                    for link in links {
                        if let Some(ref resolved) = link.target_resolved {
                            if resolved == id {
                                backlinks.push(candidate.clone());
                                break;
                            }
                        }
                    }
                }
                Err(_) => continue,
            }
        }

        Ok(backlinks)
    }

    pub fn get_graph(&self, prefix: Option<&str>) -> StoreResult<GraphResult> {
        let concepts = self.list_concepts(prefix, None, None)?;
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        for id in &concepts {
            nodes.push(Node {
                id: id.clone(),
                frontmatter: self.read_concept_frontmatter_only(id).ok(),
            });
            match self.read_concept(id) {
                Ok(concept) => {
                    let links = extract_links(id, &concept.body);
                    for link in links {
                        edges.push(Edge {
                            source: id.clone(),
                            target_raw: link.target_raw,
                            target_resolved: link.target_resolved,
                        });
                    }
                }
                Err(_) => continue,
            }
        }

        Ok(GraphResult { nodes, edges })
    }

    // --- Search ---

    /// Re-index a concept by reading it from the store and updating the search index
    /// without writing to the store. Used by file-watcher for external changes.
    pub fn reindex_concept(&self, id: &ConceptId) {
        if let Some(ref idx) = self.search_index {
            if let Ok(concept) = self.read_concept(id) {
                let _ = idx.remove_concept(id.as_str());
                let _ = idx.add_concept(&concept);
            }
        }
    }

    /// Remove a concept from the search index without modifying the store.
    pub fn remove_from_index(&self, id: &ConceptId) {
        if let Some(ref idx) = self.search_index {
            let _ = idx.remove_concept(id.as_str());
        }
    }

    pub fn search(
        &self,
        query: &str,
        type_filter: Option<&str>,
        tag_filter: Option<&str>,
    ) -> StoreResult<Vec<SearchResult>> {
        // Use tantivy search index when available
        if let Some(ref idx) = self.search_index {
            match idx.search(query, type_filter, tag_filter, 50) {
                Ok(results) => return Ok(results),
                Err(e) => {
                    tracing::warn!("search index query failed, falling back to linear scan: {e}");
                }
            }
        }

        let all_concepts = self.list_concepts(None, type_filter, tag_filter)?;
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for id in &all_concepts {
            match self.read_concept(id) {
                Ok(concept) => {
                    let title = concept.frontmatter.title.clone().unwrap_or_default();
                    let description = concept.frontmatter.description.clone().unwrap_or_default();
                    let tags = concept
                        .frontmatter
                        .tags
                        .clone()
                        .unwrap_or_default()
                        .join(" ");
                    let searchable = format!(
                        "{} {} {} {} {}",
                        id.as_str(),
                        title,
                        description,
                        tags,
                        concept.body
                    );
                    let search_lower = searchable.to_lowercase();

                    if let Some(pos) = search_lower.find(&query_lower) {
                        let score = compute_relevance_score(
                            &query_lower,
                            &searchable,
                            &title,
                            &description,
                            &tags,
                        );
                        let snippet = extract_snippet(&searchable, pos, 200);

                        // Skip body-only matches for metadata-only mode
                        let metadata_text =
                            format!("{} {} {} {}", id.as_str(), title, description, tags);
                        let metadata_lower = metadata_text.to_lowercase();
                        if !metadata_lower.contains(&query_lower) {
                            // body-only match — include for fulltext/auto
                        }

                        results.push(SearchResult {
                            concept_id: id.to_string(),
                            title: concept.frontmatter.title.clone(),
                            description: concept.frontmatter.description.clone(),
                            score,
                            snippet,
                        });
                    }
                }
                Err(_) => continue,
            }
        }

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(50);
        Ok(results)
    }
}

// --- Graph types ---

#[derive(Debug, Clone, serde::Serialize)]
pub struct Node {
    pub id: ConceptId,
    pub frontmatter: Option<Frontmatter>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Edge {
    pub source: ConceptId,
    pub target_raw: String,
    pub target_resolved: Option<ConceptId>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphResult {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct IndexReadResult {
    pub rendered: String,
    pub okf_version: Option<String>,
    pub sections: Option<Vec<IndexSection>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WriteMode {
    Create,
    Update,
    Upsert,
}

// --- Parsing utilities ---

fn parse_frontmatter(content: &str) -> Result<(Frontmatter, String), String> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return Err("no frontmatter delimiter found".to_string());
    }

    let content = &content[3..];
    let end = content
        .find("\n---")
        .ok_or_else(|| "closing frontmatter delimiter not found".to_string())?;

    let yaml_str = &content[..end];
    let rest = content[end + 4..].trim_start().to_string();

    let mut frontmatter: Frontmatter =
        serde_yaml::from_str(yaml_str).map_err(|e| format!("YAML parse error: {e}"))?;

    // Re-parse unknown keys from raw YAML to preserve them verbatim in extra
    let raw_mapping: serde_yaml::Mapping =
        serde_yaml::from_str(yaml_str).map_err(|e| format!("YAML parse error: {e}"))?;

    let known_keys = [
        "type",
        "title",
        "description",
        "resource",
        "tags",
        "timestamp",
    ];
    for (key, value) in raw_mapping {
        let key_str = key.as_str().unwrap_or("").to_string();
        if !known_keys.contains(&key_str.as_str()) {
            frontmatter.extra.insert(key, value);
        }
    }

    Ok((frontmatter, rest))
}

fn serialize_concept(frontmatter: &Frontmatter, body: &str) -> String {
    let mut yaml_mapping = serde_yaml::Mapping::new();

    yaml_mapping.insert(
        serde_yaml::Value::String("type".to_string()),
        serde_yaml::Value::String(frontmatter.r#type.clone()),
    );

    if let Some(ref title) = frontmatter.title {
        yaml_mapping.insert(
            serde_yaml::Value::String("title".to_string()),
            serde_yaml::Value::String(title.clone()),
        );
    }

    if let Some(ref desc) = frontmatter.description {
        yaml_mapping.insert(
            serde_yaml::Value::String("description".to_string()),
            serde_yaml::Value::String(desc.clone()),
        );
    }

    if let Some(ref resource) = frontmatter.resource {
        yaml_mapping.insert(
            serde_yaml::Value::String("resource".to_string()),
            serde_yaml::Value::String(resource.clone()),
        );
    }

    if let Some(ref tags) = frontmatter.tags {
        let tags_val: Vec<serde_yaml::Value> = tags
            .iter()
            .map(|t| serde_yaml::Value::String(t.clone()))
            .collect();
        yaml_mapping.insert(
            serde_yaml::Value::String("tags".to_string()),
            serde_yaml::Value::Sequence(tags_val),
        );
    }

    if let Some(ref ts) = frontmatter.timestamp {
        yaml_mapping.insert(
            serde_yaml::Value::String("timestamp".to_string()),
            serde_yaml::Value::String(ts.clone()),
        );
    }

    // Extra keys
    for (key, value) in &frontmatter.extra {
        yaml_mapping.insert(key.clone(), value.clone());
    }

    let yaml_str = serde_yaml::to_string(&yaml_mapping).unwrap_or_default();
    format!("---\n{yaml_str}---\n\n{body}")
}

fn link_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\[([^\]]*)\]\(([^)]+)\)").unwrap())
}

fn extract_links(source: &ConceptId, body: &str) -> Vec<Link> {
    let mut links = Vec::new();
    let re = link_regex();

    for cap in re.captures_iter(body) {
        let target_raw = cap[2].to_string();

        // Skip external URLs
        if target_raw.starts_with("http://") || target_raw.starts_with("https://") {
            continue;
        }

        let resolved = resolve_link(&target_raw);
        links.push(Link {
            source: source.clone(),
            target_raw,
            target_resolved: resolved,
        });
    }

    links
}

fn resolve_link(target: &str) -> Option<ConceptId> {
    let clean = target.trim_start_matches("./").trim_end_matches(".md");
    if clean.is_empty() || clean.contains("://") || clean.starts_with('#') {
        return None;
    }
    Some(ConceptId::new(clean))
}

fn index_entry_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"^\s*\*\s*\[([^\]]*)\]\(([^)]+)\)(?:\s*-\s*(.*))?")
            .unwrap()
    })
}

fn parse_index_body(body: &str) -> Option<Vec<IndexSection>> {
    let mut sections = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut current_entries = Vec::new();

    for line in body.lines() {
        if line.starts_with("# ") {
            if let Some(heading) = current_heading.take() {
                if !current_entries.is_empty() {
                    sections.push(IndexSection {
                        heading,
                        entries: std::mem::take(&mut current_entries),
                    });
                }
            }
            current_heading = Some(line[2..].trim().to_string());
        } else if let Some(cap) = index_entry_regex().captures(line) {
            current_entries.push(IndexEntry {
                title: cap[1].to_string(),
                path: cap[2].to_string(),
                description: cap.get(3).map(|m| m.as_str().to_string()),
            });
        }
    }

    if let Some(heading) = current_heading {
        if !current_entries.is_empty() {
            sections.push(IndexSection {
                heading,
                entries: current_entries,
            });
        }
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections)
    }
}

fn render_index(sections: &[IndexSection], okf_version: Option<String>) -> String {
    let mut output = String::new();
    output.push_str("---\n");
    if let Some(ver) = okf_version {
        output.push_str(&format!("okf_version: \"{ver}\"\n"));
    }
    output.push_str("---\n\n");

    for section in sections {
        output.push_str(&format!("# {}\n", section.heading));
        for entry in &section.entries {
            let desc = entry
                .description
                .as_ref()
                .map(|d| format!(" - {d}"))
                .unwrap_or_default();
            output.push_str(&format!("* [{}]({}){}\n", entry.title, entry.path, desc));
        }
        output.push('\n');
    }

    output
}

fn append_to_log(existing: &str, date: &str, entries: &[LogEntry]) -> String {
    let mut result = String::new();
    let date_heading = format!("## {date}");
    let mut inserted = false;
    let mut found_date = false;
    let mut after_first_heading = false;

    // Check if log has frontmatter
    let body = if existing.trim_start().starts_with("---") {
        // Find end of frontmatter
        if let Some(end) = existing.find("\n---") {
            result.push_str(&existing[..end + 5]);
            result.push('\n');
            existing[end + 5..].trim_start()
        } else {
            existing
        }
    } else {
        existing
    };

    // Process lines, looking for ## date headings
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") {
            after_first_heading = true;
            if trimmed == date_heading {
                found_date = true;
                result.push_str(line);
                result.push('\n');
                // Append entries under this heading
                for entry in entries {
                    if let Some(ref label) = entry.label {
                        result.push_str(&format!("- **{}** {}\n", label, entry.text));
                    } else {
                        result.push_str(&format!("- {}\n", entry.text));
                    }
                }
            } else if !found_date && !inserted {
                // Insert new date heading before this one (newest first)
                result.push_str(&date_heading);
                result.push('\n');
                for entry in entries {
                    if let Some(ref label) = entry.label {
                        result.push_str(&format!("- **{}** {}\n", label, entry.text));
                    } else {
                        result.push_str(&format!("- {}\n", entry.text));
                    }
                }
                result.push('\n');
                inserted = true;
                result.push_str(line);
                result.push('\n');
            } else {
                result.push_str(line);
                result.push('\n');
            }
        } else if !after_first_heading {
            // Content before first heading (frontmatter, initial description)
            result.push_str(line);
            result.push('\n');
        } else if found_date {
            // Under the matched date heading: skip old bullet entries, keep everything else
            if !trimmed.starts_with("- ") {
                result.push_str(line);
                result.push('\n');
            }
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    if !found_date && !inserted {
        if after_first_heading {
            result.push('\n');
        }
        result.push_str(&date_heading);
        result.push('\n');
        for entry in entries {
            if let Some(ref label) = entry.label {
                result.push_str(&format!("- **{}** {}\n", label, entry.text));
            } else {
                result.push_str(&format!("- {}\n", entry.text));
            }
        }
        result.push('\n');
    }

    result.trim().to_string() + "\n"
}

fn add_citation_to_body(body: &str, citation: &CitationInput) -> String {
    let citations_heading = "# Citations";
    let mut lines: Vec<String> = body.lines().map(|l| l.to_string()).collect();

    // Find existing citations section
    let citations_idx = lines.iter().position(|l| l.trim() == citations_heading);

    if let Some(idx) = citations_idx {
        // Count existing citations
        let mut citation_count = 0;
        for line in &lines[idx + 1..] {
            if line.trim().is_empty() {
                continue;
            }
            if line.trim().starts_with('[') {
                citation_count += 1;
            } else if line.trim().starts_with('#') && !line.trim().starts_with("## ") {
                break;
            }
        }

        let new_num = citation_count + 1;
        let new_citation = format!("[{}] [{}]({})", new_num, citation.title, citation.target);

        // Insert after the last citation line
        let mut insert_at = idx + 1;
        for i in idx + 1..lines.len() {
            if lines[i].trim().starts_with('[') {
                insert_at = i + 1;
            } else if !lines[i].trim().is_empty() {
                break;
            }
        }

        lines.insert(insert_at, new_citation);
    } else {
        // Create new citations section at the end
        if !lines.is_empty() && !lines.last().map_or(true, |l| l.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(citations_heading.to_string());
        lines.push(String::new());
        lines.push(format!("[1] [{}]({})", citation.title, citation.target));
    }

    lines.join("\n") + "\n"
}

fn compute_relevance_score(
    query: &str,
    full_text: &str,
    title: &str,
    description: &str,
    tags: &str,
) -> f64 {
    let mut score = 0.0;
    let title_lower = title.to_lowercase();
    let desc_lower = description.to_lowercase();

    // Title match is high value
    if title_lower.contains(query) {
        score += 10.0;
        // Exact title match bonus
        if title_lower == query {
            score += 20.0;
        }
    }

    // Description match is medium value
    if desc_lower.contains(query) {
        score += 5.0;
    }

    // Body match count — skip the metadata prefix: "{id} {title} {description} {tags} "
    let prefix_chars = 1 + title.chars().count() + 1 + description.chars().count() + 1 + tags.chars().count() + 1;
    if prefix_chars < full_text.chars().count() {
        let body: String = full_text.chars().skip(prefix_chars).collect();
        let body_lower = body.to_lowercase();
        let count = body_lower.matches(query).count();
        score += count as f64 * 1.0;
    }

    score
}

fn extract_snippet(text: &str, pos: usize, max_len: usize) -> String {
    let start = pos.saturating_sub(80);
    let end = std::cmp::min(start + max_len, text.len());

    let snippet = if start > 0 {
        format!("...{}", &text[start..end])
    } else {
        text[..end].to_string()
    };

    let snippet = snippet.trim();
    if end < text.len() {
        format!("{snippet}...")
    } else {
        snippet.to_string()
    }
}
