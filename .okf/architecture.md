---
type: note
title: Architecture Overview
tags:
- architecture
- rust
- mcp
---

# Architecture Overview

The OKF MCP Server is a Rust-based MCP server implementing the [OKF (Open Knowledge Format)](https://okf.ai) protocol.

## Components

- **MCP Server Layer** (`src/server.rs`) — Implements `rmcp::ServerHandler` trait for stdio transport
- **Bundle Repository** (`src/bundle/repo.rs`) — CRUD operations on knowledge bundles (frontmatter + body files)
- **File Store** (`src/bundle/fs_store.rs`) — Atomic file writes with temp+rename pattern
- **Path Safety** (`src/bundle/path_safety.rs`) — Prevents path traversal attacks
- **Validation** (`src/bundle/validate.rs`) — OKF conformance validation (3 hard rules)
- **Tools Layer** (`src/tools/`) — Read-only and write tool facades
- **Audit Log** (`src/audit.rs`) — JSONL audit trail per bundle

## Tech Stack

- **Runtime**: Tokio (async)
- **MCP SDK**: `rmcp` v0.16 with ServerHandler trait
- **Serialization**: serde + serde_json + serde_yaml
- **Schema Gen**: schemars for JSON Schema tool inputs
- **Config**: TOML via toml crate
