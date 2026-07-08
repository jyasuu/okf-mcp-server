use std::collections::HashMap;
use std::sync::Arc;

use crate::audit::AuditLog;
use crate::bundle::repo::BundleRepo;
use crate::bundle::types::*;

pub struct ReadTools {
    bundles: HashMap<String, Arc<BundleRepo>>,
    audit: Option<Arc<AuditLog>>,
}

impl ReadTools {
    pub fn new(bundles: HashMap<String, Arc<BundleRepo>>, audit: Option<Arc<AuditLog>>) -> Self {
        Self { bundles, audit }
    }

    fn get_bundle(&self, name: &str) -> Result<Arc<BundleRepo>, String> {
        self.bundles
            .get(name)
            .cloned()
            .ok_or_else(|| format!("bundle not found: {name}"))
    }

    pub fn list_bundles(&self) -> Result<Vec<BundleInfo>, String> {
        Ok(self
            .bundles
            .iter()
            .map(|(name, repo)| BundleInfo {
                name: name.clone(),
                backend: "fs".to_string(),
                path: repo.name().to_string(),
                default_branch: None,
            })
            .collect())
    }

    pub fn list_concepts(
        &self,
        bundle: &str,
        prefix: Option<&str>,
        type_filter: Option<&str>,
        tag_filter: Option<&str>,
    ) -> Result<Vec<ConceptId>, String> {
        let repo = self.get_bundle(bundle)?;
        repo.list_concepts(prefix, type_filter, tag_filter)
            .map_err(|e| e.to_string())
    }

    pub fn read_concept(&self, bundle: &str, concept_id: &str) -> Result<Concept, String> {
        let repo = self.get_bundle(bundle)?;
        let id = ConceptId::new(concept_id);
        repo.read_concept(&id).map_err(|e| e.to_string())
    }

    pub fn read_index(
        &self,
        bundle: &str,
        path: &str,
    ) -> Result<crate::bundle::repo::IndexReadResult, String> {
        let repo = self.get_bundle(bundle)?;
        repo.read_index(path).map_err(|e| e.to_string())
    }

    pub fn get_backlinks(&self, bundle: &str, concept_id: &str) -> Result<Vec<ConceptId>, String> {
        let repo = self.get_bundle(bundle)?;
        let id = ConceptId::new(concept_id);
        repo.get_backlinks(&id).map_err(|e| e.to_string())
    }

    pub fn get_graph(
        &self,
        bundle: &str,
        prefix: Option<&str>,
    ) -> Result<crate::bundle::repo::GraphResult, String> {
        let repo = self.get_bundle(bundle)?;
        repo.get_graph(prefix).map_err(|e| e.to_string())
    }

    pub fn search(
        &self,
        bundle: &str,
        query: &str,
        type_filter: Option<&str>,
        tag_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>, String> {
        let repo = self.get_bundle(bundle)?;
        repo.search(query, type_filter, tag_filter)
            .map_err(|e| e.to_string())
    }

    pub fn validate_bundle(&self, bundle: &str) -> Result<ValidationResult, String> {
        let repo = self.get_bundle(bundle)?;
        repo.validate().map_err(|e| e.to_string())
    }
}
