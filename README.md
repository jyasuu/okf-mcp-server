# okf-mcp-server

MCP (Model Context Protocol) server for browsing and managing [OKF (Open Knowledge Format)](https://github.com/anomalyco/okf) knowledge bundles.

## Features

### Read Tools
- **okf_list_bundles** — List all registered bundles
- **okf_list_concepts** — List concepts, optionally filtered by prefix, type, or tag
- **okf_read_concept** — Read a concept by ID (frontmatter + body)
- **okf_read_index** — Read or synthesize an index.md for a directory
- **okf_search** — Full-text search across concepts
- **okf_get_backlinks** — Get concepts that link to a given concept
- **okf_get_graph** — Get the full link graph for a bundle or subdirectory
- **okf_validate_bundle** — Validate a bundle against OKF conformance rules

### Write Tools
- **okf_write_concept** — Write a concept (create/update/upsert modes); supports `body_sections` with `replace` or `merge` mode for structured section editing
- **okf_delete_concept** — Delete a concept
- **okf_write_index** — Write or update an index.md with structured sections
- **okf_append_log** — Append entries to a log.md file
- **okf_add_citation** — Add a citation to a concept's # Citations section

### Write Allowlist
Path-based access control for write operations using glob patterns (`*`, `**`, `?`, `**/`). Configured per-bundle via `write_allowlist` in the config file.

### Git Tools
- **okf_git_status** — Show git status (staged, unstaged, untracked files)
- **okf_git_diff** — Show git diff for staged/unstaged changes
- **okf_git_commit** — Commit currently staged changes
- **okf_git_push** — Push to a remote repository
- **okf_git_pull** — Pull and merge from a remote
- **okf_git_create_branch** — Create and switch to a new branch

### Search
- **Tantivy full-text index** — Replaces linear scan with tantivy 0.26 for fast, scored search. Configured via `search.index` path.
- **File-watch auto-reindex** — When `search.watch = true`, the `notify` crate watches bundle directories for external `.md` file changes and automatically updates the search index (with debouncing).

### Audit Logging
All mutating tools can be audited to a local directory (configured via `audit_dir`).

## Configuration

The server reads `okf-config.toml` by default (override via `OKF_CONFIG` env var). Example:

```toml
[search]
index = ".okf-search"
watch = true

[bundles.default]
backend = "fs"
path = "bundles/default"
write_allowlist = ["tables/**", "views/**"]

[bundles.remote]
backend = "git"
path = "/path/to/repo"
remote = "https://github.com/user/repo.git"
default_branch = "main"

[audit_dir]
audit_dir = ".okf-audit"
```

Without a config file, falls back to `OKF_BUNDLE_PATH` / `OKF_BUNDLE_NAME` env vars for a single fs-backed bundle.

## Backends

- **fs** — Local filesystem store
- **git** — Git-backed store (uses `git2`), supports commit/push/pull/branch operations