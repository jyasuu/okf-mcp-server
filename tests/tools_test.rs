use std::collections::HashMap;
use std::sync::Arc;

use okf_mcp_server::bundle::fs_store::LocalFsStore;
use okf_mcp_server::bundle::repo::BundleRepo;
use okf_mcp_server::bundle::store::BundleStore;
use okf_mcp_server::bundle::types::*;
use okf_mcp_server::config::BundleBackend;
use okf_mcp_server::tools::read::ReadTools;
use okf_mcp_server::tools::write::WriteTools;

struct TestEnv {
    _dir: tempfile::TempDir,
    read: ReadTools,
    write: WriteTools,
}

fn setup() -> TestEnv {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let store: Arc<dyn BundleStore> = Arc::new(LocalFsStore::new(root.clone()));
    let repo = Arc::new(BundleRepo::new(
        "test".to_string(),
        store,
        root,
        None,
    ));

    let mut bundles = HashMap::new();
    bundles.insert("test".to_string(), repo);
    let mut backends = HashMap::new();
    backends.insert("test".to_string(), BundleBackend::Fs);

    let read = ReadTools::new(bundles.clone(), backends);
    let write = WriteTools::new(bundles, None, HashMap::new());

    TestEnv {
        _dir: dir,
        read,
        write,
    }
}

fn make_frontmatter(type_: &str, title: &str) -> Frontmatter {
    Frontmatter {
        r#type: type_.to_string(),
        title: Some(title.to_string()),
        description: None,
        resource: None,
        tags: None,
        timestamp: None,
        extra: serde_yaml::Mapping::new(),
    }
}

#[test]
fn test_write_and_read_concept() {
    let env = setup();

    let result = env
        .write
        .write_concept(
            "test",
            "tables/orders",
            make_frontmatter("Table", "Orders"),
            "# Orders\n\nOrder data.".to_string(),
            None,
            None,
            "upsert",
        )
        .unwrap();

    assert_eq!(result.id.to_string(), "tables/orders");
    assert_eq!(result.frontmatter.title.as_deref(), Some("Orders"));

    let read = env.read.read_concept("test", "tables/orders").unwrap();
    assert_eq!(read.id.to_string(), "tables/orders");
    assert_eq!(read.frontmatter.r#type, "Table");
    assert!(read.body.contains("Order data."));
}

#[test]
fn test_read_concept_not_found() {
    let env = setup();

    let result = env.read.read_concept("test", "nonexistent");
    assert!(result.is_err());
}

#[test]
fn test_list_concepts_all() {
    let env = setup();

    for (id, title) in &[
        ("tables/orders", "Orders"),
        ("tables/customers", "Customers"),
        ("views/revenue", "Revenue"),
    ] {
        env.write
            .write_concept(
                "test",
                id,
                make_frontmatter("Table", title),
                format!("# {title}"),
                None,
                None,
                "upsert",
            )
            .unwrap();
    }

    let all = env.read.list_concepts("test", None, None, None).unwrap();
    assert_eq!(all.len(), 3);
}

#[test]
fn test_list_concepts_by_prefix() {
    let env = setup();

    for (id, title) in &[
        ("tables/orders", "Orders"),
        ("tables/customers", "Customers"),
        ("views/revenue", "Revenue"),
    ] {
        env.write
            .write_concept(
                "test",
                id,
                make_frontmatter("Table", title),
                format!("# {title}"),
                None,
                None,
                "upsert",
            )
            .unwrap();
    }

    let tables = env
        .read
        .list_concepts("test", Some("tables"), None, None)
        .unwrap();
    assert_eq!(tables.len(), 2);
}

#[test]
fn test_list_concepts_by_type() {
    let env = setup();

    env.write
        .write_concept(
            "test",
            "orders",
            make_frontmatter("Table", "Orders"),
            "body".to_string(),
            None,
            None,
            "upsert",
        )
        .unwrap();

    env.write
        .write_concept(
            "test",
            "revenue_view",
            make_frontmatter("View", "Revenue"),
            "body".to_string(),
            None,
            None,
            "upsert",
        )
        .unwrap();

    let tables = env
        .read
        .list_concepts("test", None, Some("Table"), None)
        .unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].to_string(), "orders");

    let views = env
        .read
        .list_concepts("test", None, Some("View"), None)
        .unwrap();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].to_string(), "revenue_view");
}

#[test]
fn test_search_concepts() {
    let env = setup();

    for (id, title, desc) in &[
        ("orders", "Orders", "Order processing system"),
        ("customers", "Customers", "Customer records"),
        ("revenue", "Revenue", "Revenue analysis view"),
    ] {
        let mut fm = make_frontmatter("Table", title);
        fm.description = Some(desc.to_string());
        env.write
            .write_concept("test", id, fm, format!("# {title}\n\n{desc}."), None, None, "upsert")
            .unwrap();
    }

    let results = env.read.search("test", "orders", None, None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].concept_id, "orders");

    let results = env.read.search("test", "Customer", None, None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].concept_id, "customers");
}

#[test]
fn test_search_by_type() {
    let env = setup();

    env.write
        .write_concept(
            "test",
            "orders",
            make_frontmatter("Table", "Orders"),
            "Order data.".to_string(),
            None,
            None,
            "upsert",
        )
        .unwrap();

    env.write
        .write_concept(
            "test",
            "revenue",
            make_frontmatter("View", "Revenue"),
            "Revenue data.".to_string(),
            None,
            None,
            "upsert",
        )
        .unwrap();

    let results = env
        .read
        .search("test", "data", Some("Table"), None)
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].concept_id, "orders");
}

#[test]
fn test_delete_concept() {
    let env = setup();

    env.write
        .write_concept(
            "test",
            "to_delete",
            make_frontmatter("Type", "Delete Me"),
            "body".to_string(),
            None,
            None,
            "upsert",
        )
        .unwrap();

    assert_eq!(
        env.read.list_concepts("test", None, None, None).unwrap().len(),
        1
    );

    let deleted = env.write.delete_concept("test", "to_delete").unwrap();
    assert!(deleted);

    assert_eq!(
        env.read.list_concepts("test", None, None, None).unwrap().len(),
        0
    );
}

#[test]
fn test_delete_concept_not_found() {
    let env = setup();

    let result = env.write.delete_concept("test", "nonexistent");
    assert!(result.is_err());
}

#[test]
fn test_get_backlinks() {
    let env = setup();

    env.write
        .write_concept(
            "test",
            "target",
            make_frontmatter("Type", "Target"),
            "Target concept.".to_string(),
            None,
            None,
            "upsert",
        )
        .unwrap();

    env.write
        .write_concept(
            "test",
            "source",
            make_frontmatter("Type", "Source"),
            "This links to [target](/target.md).".to_string(),
            None,
            None,
            "upsert",
        )
        .unwrap();

    let backlinks = env.read.get_backlinks("test", "target").unwrap();
    assert_eq!(backlinks.len(), 1);
    assert_eq!(backlinks[0].to_string(), "source");
}

#[test]
fn test_get_backlinks_none() {
    let env = setup();

    env.write
        .write_concept(
            "test",
            "standalone",
            make_frontmatter("Type", "Standalone"),
            "No links here.".to_string(),
            None,
            None,
            "upsert",
        )
        .unwrap();

    let backlinks = env.read.get_backlinks("test", "standalone").unwrap();
    assert!(backlinks.is_empty());
}

#[test]
fn test_write_modes() {
    let env = setup();

    env.write
        .write_concept(
            "test",
            "concept",
            make_frontmatter("Type", "V1"),
            "v1 body".to_string(),
            None,
            None,
            "create",
        )
        .unwrap();

    // "create" should fail when concept already exists
    let result = env.write.write_concept(
        "test",
        "concept",
        make_frontmatter("Type", "V2"),
        "v2 body".to_string(),
        None,
        None,
        "create",
    );
    assert!(result.is_err());

    // "update" should succeed on existing concept
    let result = env.write.write_concept(
        "test",
        "concept",
        make_frontmatter("Type", "V3"),
        "v3 body".to_string(),
        None,
        None,
        "update",
    );
    assert!(result.is_ok());

    let read = env.read.read_concept("test", "concept").unwrap();
    assert_eq!(read.frontmatter.title.as_deref(), Some("V3"));

    // "update" should fail on nonexistent concept
    let result = env.write.write_concept(
        "test",
        "missing",
        make_frontmatter("Type", "Nope"),
        "body".to_string(),
        None,
        None,
        "update",
    );
    assert!(result.is_err());

    // "upsert" should always succeed
    let result = env.write.write_concept(
        "test",
        "missing",
        make_frontmatter("Type", "Upserted"),
        "body".to_string(),
        None,
        None,
        "upsert",
    );
    assert!(result.is_ok());
}

#[test]
fn test_bundle_not_found() {
    let env = setup();

    assert!(env.read.read_concept("nonexistent", "x").is_err());
    assert!(env.read.list_concepts("nonexistent", None, None, None).is_err());
    assert!(env.read.search("nonexistent", "q", None, None).is_err());
    assert!(env.read.get_backlinks("nonexistent", "x").is_err());
    assert!(env
        .write
        .write_concept(
            "nonexistent",
            "x",
            make_frontmatter("T", "T"),
            "b".to_string(),
            None,
            None,
            "upsert",
        )
        .is_err());
    assert!(env.write.delete_concept("nonexistent", "x").is_err());
}

#[test]
fn test_write_with_body_sections() {
    let env = setup();

    let sections = vec![
        BodySection {
            heading: String::new(),
            content: "Intro paragraph.".to_string(),
        },
        BodySection {
            heading: "Schema".to_string(),
            content: "| col | type |\n|-----|------|\n| id | INT64 |".to_string(),
        },
    ];

    let result = env
        .write
        .write_concept(
            "test",
            "sectioned",
            make_frontmatter("Table", "Sectioned"),
            String::new(),
            Some(sections),
            None,
            "upsert",
        )
        .unwrap();

    assert!(result.body.contains("Intro paragraph."));
    assert!(result.body.contains("## Schema"));

    let read = env.read.read_concept("test", "sectioned").unwrap();
    assert!(read.body.contains("## Schema"));
}

#[test]
fn test_temp_dir_cleanup() {
    let path;
    {
        let env = setup();
        path = env._dir.path().to_path_buf();
        assert!(path.exists());
        env.write
            .write_concept(
                "test",
                "item",
                make_frontmatter("Type", "Item"),
                "body".to_string(),
                None,
                None,
                "upsert",
            )
            .unwrap();
        assert!(path.exists());
    }
    assert!(!path.exists(), "temp dir should be removed after drop");
}
