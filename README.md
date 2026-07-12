# okf-mcp-server

MCP (Model Context Protocol) server for browsing and managing [OKF (Open Knowledge Format)](https://github.com/anomalyco/okf) knowledge bundles.

## Features

### Read Tools
- **okf_list_bundles** — List all registered bundles
- **okf_list_concepts** — List concepts, optionally filtered by prefix, `concept_type`, or tag
- **okf_read_concept** — Read a concept by ID (frontmatter + body)
- **okf_read_index** — Read or synthesize an index.md for a directory
- **okf_search** — Full-text search across concepts
- **okf_get_backlinks** — Get concepts that link to a given concept
- **okf_get_graph** — Get the full link graph for a bundle or subdirectory
- **okf_validate_bundle** — Validate a bundle against OKF conformance rules

### Write Tools
- **okf_write_concept** — Write a concept (create/update/upsert modes). Takes a `data` JSON string with fields: `type` (required), `title`, `description`, `resource`, `tags`, `timestamp`, `body`, `body_sections` (array of `{heading, content}`), `body_section_mode` (replace/merge), `mode` (create/update/upsert)
- **okf_delete_concept** — Delete a concept
- **okf_write_index** — Write or update an index.md with structured sections
- **okf_append_log** — Append entries to a log.md file
- **okf_add_citation** — Add a citation to a concept's `# Citations` section

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
- **Tantivy full-text index** — Uses tantivy 0.26 with CJK/Chinese text tokenization (cang-jie/jieba) for fast, scored search. Configured via `search.index` path. Batched commits (`COMMIT_BATCH_SIZE=50`) for write performance.
- **File-watch auto-reindex** — When `search.watch = true`, the `notify` crate watches bundle directories for external `.md` file changes and automatically updates the search index (with debouncing and graceful shutdown).

### Path Safety
- Path traversal protection: rejects `..` segments, absolute paths, and null bytes
- Symlink safety: `verify_parent_chain()` checks parent directories for symlinks pointing outside the bundle root
- Reserved filename enforcement (`index.md`, `log.md`)

### Audit Logging
All mutating tools are audited to a single `audit.jsonl` file in the configured `audit_dir`.

## Running

The server uses stdin/stdout transport (no CLI args). Configure via environment variables or a TOML config file:

| Env Var | Description |
|---------|-------------|
| `OKF_CONFIG` | Path to config file (default: `okf-config.toml`) |
| `OKF_BUNDLE_PATH` | Fallback: path to a single bundle directory |
| `OKF_BUNDLE_NAME` | Fallback: name for the single bundle (default: `default`) |
| `RUST_LOG` | Log level (default: `info`) |

## Configuration

The server reads `okf-config.toml` by default (override via `OKF_CONFIG` env var). Example:

```toml
audit_dir = ".okf-audit"

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
branch_policy = "session-branch"

[bundles.remote.auth]
ssh_key = "/path/to/deploy_key"
```

Without a config file, falls back to `OKF_BUNDLE_PATH` / `OKF_BUNDLE_NAME` env vars for a single fs-backed bundle.

## Backends

- **fs** — Local filesystem store
- **git** — Git-backed store (uses `git2`), supports commit/push/pull/branch operations

## Testing

- **41 tests total**: 6 unit tests, 20 integration tests, 15 tool-layer tests
- **MCP E2E tests**: `tests/test_mcp.sh` — 18 assertions covering write, read, search, list, validate, backlinks, and delete via the MCP JSON-RPC protocol
- **Release workflow**: `.github/workflows/release.yml` — matrix build for Linux (x86_64), macOS (aarch64), and Windows (x86_64)