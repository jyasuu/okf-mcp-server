use std::collections::HashMap;
use std::sync::Arc;

use regex::Regex;

use crate::audit::AuditLog;
use crate::bundle::path_safety::PathChecker;
use crate::bundle::repo::{BundleRepo, WriteMode};
use crate::bundle::types::*;

pub struct WriteTools {
    bundles: HashMap<String, Arc<BundleRepo>>,
    audit: Option<Arc<AuditLog>>,
    allowlists: HashMap<String, Option<Vec<String>>>,
}

impl WriteTools {
    pub fn new(
        bundles: HashMap<String, Arc<BundleRepo>>,
        audit: Option<Arc<AuditLog>>,
        allowlists: HashMap<String, Option<Vec<String>>>,
    ) -> Self {
        Self {
            bundles,
            audit,
            allowlists,
        }
    }

    fn get_bundle(&self, name: &str) -> Result<Arc<BundleRepo>, String> {
        self.bundles
            .get(name)
            .cloned()
            .ok_or_else(|| format!("bundle not found: {name}"))
    }

    fn check_allowlist(&self, bundle: &str, target_path: &str) -> Result<(), String> {
        let patterns = match self.allowlists.get(bundle) {
            Some(Some(p)) => p,
            _ => return Ok(()), // no allowlist = allow all
        };

        if patterns.is_empty() {
            return Ok(());
        }

        let allowed = patterns.iter().any(|pat| {
            let re = match glob_to_regex(pat) {
                Ok(r) => r,
                Err(_) => return false,
            };
            re.is_match(target_path)
        });

        if !allowed {
            return Err(format!(
                "path '{target_path}' is not allowed by write_allowlist patterns: [{}]",
                patterns.join(", ")
            ));
        }

        Ok(())
    }

    fn audit_ok(&self, tool: &str, bundle: &str, target: &str, summary: &str) {
        if let Some(ref audit) = self.audit {
            let _ = audit.record_ok(tool, bundle, target, summary);
        }
    }

    fn audit_error(&self, tool: &str, bundle: &str, target: &str, summary: &str, error: &str) {
        if let Some(ref audit) = self.audit {
            let _ = audit.record_error(tool, bundle, target, summary, error);
        }
    }

    pub fn write_concept(
        &self,
        bundle: &str,
        concept_id: &str,
        frontmatter: Frontmatter,
        body: String,
        body_sections: Option<Vec<BodySection>>,
        body_section_mode: Option<String>,
        mode: &str,
    ) -> Result<Concept, String> {
        let repo = self.get_bundle(bundle)?;

        // Path safety check
        PathChecker::check_concept_id(concept_id).map_err(|e| e.to_string())?;

        // Reserved filename check
        let path = format!("{concept_id}.md");
        if PathChecker::is_reserved_filename(&path) {
            return Err(format!("concept ID uses reserved filename: {concept_id}"));
        }

        // Allowlist check
        self.check_allowlist(bundle, &path)?;

        // Type validation
        if frontmatter.r#type.trim().is_empty() {
            return Err("frontmatter type is required and must be non-empty".to_string());
        }

        let write_mode = match mode {
            "create" => WriteMode::Create,
            "update" => WriteMode::Update,
            "upsert" => WriteMode::Upsert,
            _ => {
                return Err(format!(
                    "invalid mode: {mode}, expected create/update/upsert"
                ))
            }
        };

        // Resolve body: body_sections takes precedence over raw body string
        let resolved_body = if let Some(sections) = body_sections {
            let section_mode = body_section_mode.as_deref().unwrap_or("replace");
            match section_mode {
                "merge" => {
                    let existing_sections = if write_mode != WriteMode::Create {
                        let id = ConceptId::new(concept_id);
                        match repo.read_concept(&id) {
                            Ok(existing) => parse_body_sections(&existing.body),
                            Err(_) => Vec::new(),
                        }
                    } else {
                        Vec::new()
                    };
                    let merged = merge_body_sections(&existing_sections, &sections);
                    render_body_sections(&merged)
                }
                "replace" => render_body_sections(&sections),
                other => {
                    return Err(format!(
                        "invalid body_section_mode: {other}, expected replace/merge"
                    ))
                }
            }
        } else {
            body
        };

        let id = ConceptId::new(concept_id);
        let concept = Concept {
            id: id.clone(),
            frontmatter,
            body: resolved_body,
        };

        let result = repo.write_concept(concept, write_mode).map_err(|e| {
            self.audit_error(
                "okf_write_concept",
                bundle,
                concept_id,
                &format!("mode={mode}"),
                &e.to_string(),
            );
            e.to_string()
        })?;

        self.audit_ok(
            "okf_write_concept",
            bundle,
            concept_id,
            &format!("mode={mode}"),
        );
        Ok(result)
    }

    pub fn delete_concept(&self, bundle: &str, concept_id: &str) -> Result<bool, String> {
        let repo = self.get_bundle(bundle)?;

        PathChecker::check_concept_id(concept_id).map_err(|e| e.to_string())?;

        let path = format!("{concept_id}.md");
        self.check_allowlist(bundle, &path)?;

        let id = ConceptId::new(concept_id);
        let result = repo.delete_concept(&id).map_err(|e| {
            self.audit_error("okf_delete_concept", bundle, concept_id, "", &e.to_string());
            e.to_string()
        })?;

        self.audit_ok("okf_delete_concept", bundle, concept_id, "");
        Ok(result)
    }

    pub fn write_index(
        &self,
        bundle: &str,
        path: &str,
        sections: Vec<IndexSection>,
        okf_version: Option<String>,
    ) -> Result<String, String> {
        let repo = self.get_bundle(bundle)?;

        let index_path = if path.is_empty() || path == "/" {
            "index.md".to_string()
        } else {
            format!(
                "{}/index.md",
                path.trim_start_matches('/').trim_end_matches('/')
            )
        };
        self.check_allowlist(bundle, &index_path)?;

        let data = IndexData {
            sections,
            okf_version,
        };
        let result = repo.write_index(path, data).map_err(|e| {
            self.audit_error("okf_write_index", bundle, path, "", &e.to_string());
            e.to_string()
        })?;

        self.audit_ok("okf_write_index", bundle, path, "");
        Ok(result)
    }

    pub fn append_log(
        &self,
        bundle: &str,
        path: &str,
        date: &str,
        entries: Vec<LogEntry>,
    ) -> Result<String, String> {
        let repo = self.get_bundle(bundle)?;

        let log_path = if path.is_empty() || path == "/" {
            "log.md".to_string()
        } else {
            format!(
                "{}/log.md",
                path.trim_start_matches('/').trim_end_matches('/')
            )
        };
        self.check_allowlist(bundle, &log_path)?;

        let result = repo.append_log(path, date, &entries).map_err(|e| {
            self.audit_error(
                "okf_append_log",
                bundle,
                path,
                &format!("date={date}"),
                &e.to_string(),
            );
            e.to_string()
        })?;

        self.audit_ok("okf_append_log", bundle, path, &format!("date={date}"));
        Ok(result)
    }

    pub fn add_citation(
        &self,
        bundle: &str,
        concept_id: &str,
        title: &str,
        target: &str,
    ) -> Result<String, String> {
        let repo = self.get_bundle(bundle)?;

        PathChecker::check_concept_id(concept_id).map_err(|e| e.to_string())?;

        let path = format!("{concept_id}.md");
        self.check_allowlist(bundle, &path)?;

        let id = ConceptId::new(concept_id);
        let citation = CitationInput {
            title: title.to_string(),
            target: target.to_string(),
        };

        let result = repo.add_citation(&id, &citation).map_err(|e| {
            self.audit_error(
                "okf_add_citation",
                bundle,
                concept_id,
                &format!("title={title}"),
                &e.to_string(),
            );
            e.to_string()
        })?;

        self.audit_ok(
            "okf_add_citation",
            bundle,
            concept_id,
            &format!("title={title}"),
        );
        Ok(result)
    }
}

fn glob_to_regex(pattern: &str) -> Result<Regex, String> {
    let mut re_str = String::with_capacity(pattern.len() + 4);
    re_str.push('^');

    let mut chars = pattern.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next(); // consume second *
                    if chars.peek() == Some(&'/') {
                        chars.next(); // consume /
                                      // **/ - optionally match anything up to /
                        re_str.push_str("(?:.+/)?");
                    } else {
                        re_str.push_str(".*");
                    }
                } else {
                    re_str.push_str("[^/]*");
                }
            }
            '?' => re_str.push_str("[^/]"),
            '+' | '.' | '^' | '$' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '\\' => {
                re_str.push('\\');
                re_str.push(c);
            }
            _ => re_str.push(c),
        }
    }

    re_str.push('$');

    Regex::new(&re_str).map_err(|e| format!("invalid glob pattern '{pattern}': {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_to_regex_exact() {
        let re = glob_to_regex("tables/orders.md").unwrap();
        assert!(re.is_match("tables/orders.md"));
        assert!(!re.is_match("tables/orders2.md"));
    }

    #[test]
    fn test_glob_to_regex_wildcard() {
        let re = glob_to_regex("tables/*").unwrap();
        assert!(re.is_match("tables/orders.md"));
        assert!(re.is_match("tables/customers"));
        assert!(!re.is_match("tables/sub/orders.md"));
    }

    #[test]
    fn test_glob_to_regex_globstar() {
        let re = glob_to_regex("datasets/**").unwrap();
        assert!(re.is_match("datasets/foo.md"));
        assert!(re.is_match("datasets/sub/bar.md"));
        assert!(!re.is_match("other/file.md"));
    }

    #[test]
    fn test_glob_to_regex_globstar_slash() {
        let re = glob_to_regex("datasets/**/*.md").unwrap();
        assert!(re.is_match("datasets/foo.md"));
        assert!(re.is_match("datasets/sub/bar.md"));
        assert!(!re.is_match("datasets/foo.txt"));
    }

    #[test]
    fn test_allowlist_write_concept_allowed() {
        let tools = WriteTools::new(
            HashMap::new(),
            None,
            [("test".to_string(), Some(vec!["tables/**".to_string()]))].into(),
        );
        assert!(tools.check_allowlist("test", "tables/orders.md").is_ok());
        assert!(tools.check_allowlist("test", "views/revenue.md").is_err());
    }

    #[test]
    fn test_allowlist_no_patterns_allows_all() {
        let tools = WriteTools::new(HashMap::new(), None, [("test".to_string(), None)].into());
        assert!(tools.check_allowlist("test", "anything.md").is_ok());
    }
}
