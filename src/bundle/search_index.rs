use std::path::Path;
use std::sync::Mutex;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::{doc, Index, IndexWriter, TantivyDocument};

use crate::bundle::types::{Concept, SearchResult};

pub struct SearchIndex {
    index: Index,
    writer: Mutex<IndexWriter>,
    id_field: Field,
    title_field: Field,
    description_field: Field,
    type_field: Field,
    tags_field: Field,
    body_field: Field,
    all_text_field: Field,
}

impl SearchIndex {
    pub fn new(index_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(index_dir)
            .map_err(|e| format!("failed to create search index dir: {e}"))?;

        let mut schema_builder = Schema::builder();
        let id_field = schema_builder.add_text_field("id", STRING | STORED);
        let title_field = schema_builder.add_text_field("title", TEXT | STORED);
        let description_field = schema_builder.add_text_field("description", TEXT | STORED);
        let type_field = schema_builder.add_text_field("type", STRING | STORED);
        let tags_field = schema_builder.add_text_field("tags", STRING | STORED);
        let body_field = schema_builder.add_text_field("body", TEXT | STORED);
        let all_text_field = schema_builder.add_text_field("all_text", TEXT);
        let schema = schema_builder.build();

        let index = Index::create_in_dir(index_dir, schema.clone())
            .map_err(|e| format!("failed to create tantivy index: {e}"))?;

        let writer = index
            .writer(50_000_000)
            .map_err(|e| format!("failed to create index writer: {e}"))?;

        Ok(Self {
            index,
            writer: Mutex::new(writer),
            id_field,
            title_field,
            description_field,
            type_field,
            tags_field,
            body_field,
            all_text_field,
        })
    }

    pub fn add_concept(&self, concept: &Concept) -> Result<(), String> {
        let all_text = format!(
            "{} {} {} {} {}",
            concept.id.as_str(),
            concept.frontmatter.title.as_deref().unwrap_or(""),
            concept.frontmatter.description.as_deref().unwrap_or(""),
            concept
                .frontmatter
                .tags
                .as_ref()
                .map(|t| t.join(" "))
                .unwrap_or_default(),
            concept.body,
        );

        let tags_str = concept
            .frontmatter
            .tags
            .as_ref()
            .map(|t| t.join(" "))
            .unwrap_or_default();

        let title_str = concept.frontmatter.title.as_deref().unwrap_or("");
        let desc_str = concept.frontmatter.description.as_deref().unwrap_or("");

        let mut writer = self.writer.lock().unwrap();
        writer
            .add_document(doc!(
                self.id_field => concept.id.as_str(),
                self.title_field => title_str,
                self.description_field => desc_str,
                self.type_field => concept.frontmatter.r#type.as_str(),
                self.tags_field => tags_str,
                self.body_field => concept.body.as_str(),
                self.all_text_field => all_text,
            ))
            .map_err(|e| format!("failed to index concept: {e}"))?;

        writer
            .commit()
            .map_err(|e| format!("failed to commit index: {e}"))?;

        Ok(())
    }

    pub fn remove_concept(&self, id: &str) -> Result<(), String> {
        let mut writer = self.writer.lock().unwrap();
        writer.delete_term(tantivy::Term::from_field_text(
            self.id_field,
            id,
        ));
        writer
            .commit()
            .map_err(|e| format!("failed to commit index: {e}"))?;
        Ok(())
    }

    pub fn search(
        &self,
        query: &str,
        type_filter: Option<&str>,
        tag_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchResult>, String> {
        let reader = self
            .index
            .reader_builder()
            .reload_policy(tantivy::ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| format!("failed to create index reader: {e}"))?;

        let searcher = reader.searcher();

        let query_parser = QueryParser::for_index(&self.index, vec![
            self.all_text_field,
            self.title_field,
            self.description_field,
        ]);

        let query_str = if query.trim().is_empty() {
            "*".to_string()
        } else {
            query.to_string()
        };

        let parsed_query = query_parser
            .parse_query(&query_str)
            .map_err(|e| format!("failed to parse query: {e}"))?;

        let top_docs = searcher
            .search(&parsed_query, &TopDocs::with_limit(limit).order_by_score())
            .map_err(|e| format!("search failed: {e}"))?;

        let mut results: Vec<SearchResult> = Vec::new();
        for (score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher
                .doc::<TantivyDocument>(doc_address)
                .map_err(|e| format!("failed to retrieve doc: {e}"))?;

            let concept_id = doc
                .get_first(self.id_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Apply filters
            if let Some(tf) = type_filter {
                let doc_type = doc
                    .get_first(self.type_field)
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if doc_type != tf {
                    continue;
                }
            }

            if let Some(tag) = tag_filter {
                let doc_tags = doc
                    .get_first(self.tags_field)
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !doc_tags.split(' ').any(|t| t == tag) {
                    continue;
                }
            }

            let title = doc
                .get_first(self.title_field)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let description = doc
                .get_first(self.description_field)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            // Extract snippet from body
            let body = doc
                .get_first(self.body_field)
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let snippet = extract_snippet_text(body, &query_str, 200);

            results.push(SearchResult {
                concept_id,
                title,
                description,
                score: score as f64,
                snippet,
            });
        }

        Ok(results)
    }
}

fn extract_snippet_text(text: &str, query: &str, max_len: usize) -> String {
    let query_lower = query.to_lowercase();
    if let Some(pos) = text.to_lowercase().find(&query_lower) {
        let start = pos.saturating_sub(80);
        let end = (pos + query.len() + (max_len - query.len().min(max_len))).min(text.len());
        let snippet = &text[start..end];
        if start > 0 {
            format!("...{}", snippet)
        } else {
            snippet.to_string()
        }
    } else {
        text.chars().take(max_len).collect()
    }
}
