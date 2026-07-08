use serde::{Deserialize, Serialize};
use serde_yaml::Mapping;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConceptId(pub String);

impl ConceptId {
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into().trim().to_string();
        let id = id.trim_start_matches('/').to_string();
        let id = id.strip_suffix(".md").unwrap_or(&id).to_string();
        ConceptId(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn to_path(&self) -> String {
        format!("{}.md", self.0)
    }
}

impl fmt::Display for ConceptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frontmatter {
    #[serde(rename = "type")]
    pub r#type: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub resource: Option<String>,
    pub tags: Option<Vec<String>>,
    pub timestamp: Option<String>,
    #[serde(flatten)]
    pub extra: Mapping,
}

impl Frontmatter {
    pub fn validate(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        if self.r#type.trim().is_empty() {
            errors.push(ValidationError::EmptyType);
        }
        errors
    }

    pub fn warnings(&self) -> Vec<ValidationWarning> {
        let mut warnings = Vec::new();
        if self.title.is_none() {
            warnings.push(ValidationWarning::MissingField("title"));
        }
        if self.description.is_none() {
            warnings.push(ValidationWarning::MissingField("description"));
        }
        if self.tags.is_none() {
            warnings.push(ValidationWarning::MissingField("tags"));
        }
        if self.timestamp.is_none() {
            warnings.push(ValidationWarning::MissingField("timestamp"));
        }
        warnings
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Concept {
    pub id: ConceptId,
    pub frontmatter: Frontmatter,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub source: ConceptId,
    pub target_raw: String,
    pub target_resolved: Option<ConceptId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub title: String,
    pub path: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexSection {
    pub heading: String,
    pub entries: Vec<IndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub label: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitationInput {
    pub title: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexData {
    pub sections: Vec<IndexSection>,
    pub okf_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleInfo {
    pub name: String,
    pub backend: String,
    pub path: String,
    pub default_branch: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ValidationError {
    EmptyType,
    MissingFrontmatter,
    MalformedYaml(String),
    ReservedFilename(String),
    PathTraversal(String),
    OkfVersionOnNonRoot(String),
    IoError(String),
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationError::EmptyType => write!(f, "frontmatter type is empty"),
            ValidationError::MissingFrontmatter => {
                write!(f, "missing or unparseable YAML frontmatter")
            }
            ValidationError::MalformedYaml(e) => write!(f, "malformed YAML frontmatter: {e}"),
            ValidationError::ReservedFilename(name) => {
                write!(f, "concept ID uses reserved filename: {name}")
            }
            ValidationError::PathTraversal(p) => write!(f, "path traversal detected: {p}"),
            ValidationError::OkfVersionOnNonRoot(p) => {
                write!(f, "okf_version present in non-root index.md: {p}")
            }
            ValidationError::IoError(e) => write!(f, "I/O error: {e}"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ValidationWarning {
    MissingField(&'static str),
    UnknownType(String),
    UnknownExtraKeys(Vec<String>),
    BrokenLink(String),
    MissingIndexMd(String),
    InvalidLogFormat(String),
}

impl std::fmt::Display for ValidationWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationWarning::MissingField(field) => {
                write!(f, "missing recommended frontmatter field: {field}")
            }
            ValidationWarning::UnknownType(t) => write!(f, "unknown type value: {t}"),
            ValidationWarning::UnknownExtraKeys(keys) => {
                write!(f, "unrecognized frontmatter keys: {}", keys.join(", "))
            }
            ValidationWarning::BrokenLink(l) => write!(f, "broken link: {l}"),
            ValidationWarning::MissingIndexMd(d) => write!(f, "missing index.md in directory: {d}"),
            ValidationWarning::InvalidLogFormat(d) => {
                write!(f, "log.md entry not under valid date heading: {d}")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub concept_id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub score: f64,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BodySection {
    pub heading: String,
    pub content: String,
}

pub fn render_body_sections(sections: &[BodySection]) -> String {
    let mut parts = Vec::new();
    for section in sections {
        if section.heading.is_empty() {
            if !section.content.is_empty() {
                parts.push(section.content.clone());
            }
        } else {
            let rendered = format!("## {}\n{}", section.heading, section.content);
            parts.push(rendered);
        }
    }
    parts.join("\n\n")
}

pub fn parse_body_sections(body: &str) -> Vec<BodySection> {
    let mut sections = Vec::new();
    let mut current_heading = String::new();
    let mut current_lines: Vec<&str> = Vec::new();

    for line in body.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            if !current_heading.is_empty() || !current_lines.is_empty() {
                sections.push(BodySection {
                    heading: std::mem::take(&mut current_heading),
                    content: current_lines.join("\n").trim().to_string(),
                });
                current_lines.clear();
            }
            current_heading = h.to_string();
        } else {
            current_lines.push(line);
        }
    }

    if !current_heading.is_empty() || !current_lines.is_empty() {
        sections.push(BodySection {
            heading: current_heading,
            content: current_lines.join("\n").trim().to_string(),
        });
    }

    sections
}

pub fn merge_body_sections(
    existing: &[BodySection],
    incoming: &[BodySection],
) -> Vec<BodySection> {
    let mut result: Vec<BodySection> = Vec::new();
    let mut replaced: std::collections::HashSet<&str> = std::collections::HashSet::new();

    for section in existing {
        if let Some(incoming_section) = incoming.iter().find(|s| s.heading == section.heading) {
            result.push(incoming_section.clone());
            replaced.insert(incoming_section.heading.as_str());
        } else {
            result.push(section.clone());
        }
    }

    for section in incoming {
        if !replaced.contains(section.heading.as_str()) {
            result.push(section.clone());
        }
    }

    result
}


