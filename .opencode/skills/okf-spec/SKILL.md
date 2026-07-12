---
name: okf-spec
description: Reference for Open Knowledge Format (OKF) v0.1. Use when creating, reading, or understanding OKF knowledge bundles, concept documents, index files, log files, or citations. Trigger on "what is okf", "okf spec", "okf format", "knowledge bundle", "concept document".
---

# Open Knowledge Format (OKF) v0.1

OKF is an open, human- and agent-friendly format for representing knowledge — the metadata, context, and curated insight that surrounds data and systems.

It is a directory of markdown files with YAML frontmatter. No schema registry, no central authority, no required tooling.

## Terminology

| Term | Definition |
|------|-----------|
| **Knowledge Bundle** | A self-contained, hierarchical collection of knowledge documents. The unit of distribution. |
| **Concept** | A single unit of knowledge within a bundle. One markdown file = one concept. |
| **Concept ID** | The path of the concept's file within the bundle, with `.md` removed. E.g. `tables/users.md` → `tables/users`. |
| **Frontmatter** | YAML metadata block delimited by `---` at the top of a file. |
| **Body** | Everything in the file after the frontmatter. |
| **Link** | A markdown link from one concept to another, expressing relationships. |
| **Citation** | A link from a concept to an external source supporting a claim. |

## Bundle Structure

```
bundle/
├── index.md                      # Optional. Directory listing.
├── log.md                        # Optional. Chronological update history.
├── <concept>.md                  # A concept at the bundle root.
└── <subdirectory>/
    ├── index.md
    ├── <concept>.md
    └── <subdirectory>/
        └── …
```

Reserved filenames (MUST NOT be used for concept documents):
- `index.md` — Directory listing
- `log.md` — Update history

A bundle MAY be distributed as a git repository (recommended), a tarball/zip, or a subdirectory within a larger repo.

## Concept Documents

Every concept is a UTF-8 markdown file with two parts:

1. **YAML frontmatter** — delimited by `---`
2. **Markdown body** — free-form content

### Frontmatter

```yaml
---
type: <Type name>                  # REQUIRED
title: <Optional display name>
description: <Optional one-line summary>
resource: <Optional canonical URI for the underlying asset>
tags: [<tag>, <tag>, …]
timestamp: <ISO 8601 datetime>
# … other producer-defined key/value pairs
---
```

**Required:**
- `type` — Short string identifying the kind of concept (e.g. `BigQuery Table`, `API Endpoint`, `Metric`, `Playbook`). Not registered centrally. Consumers MUST tolerate unknown types.

**Recommended:**
- `title` — Human-readable display name
- `description` — Single sentence summary
- `resource` — URI uniquely identifying the underlying asset
- `tags` — Short strings for cross-cutting categorization
- `timestamp` — ISO 8601 datetime of last meaningful change

Producers MAY include any additional keys. Consumers SHOULD preserve unknown keys and MUST NOT reject documents with unrecognized fields.

### Body

Standard markdown. Conventional section headings (not required):

| Heading | Purpose |
|---------|---------|
| `# Schema` | Structured description of an asset's columns/fields |
| `# Examples` | Concrete usage examples |
| `# Citations` | External sources backing claims |

### Example: Concept bound to a resource

```markdown
---
type: BigQuery Table
title: Customer Orders
description: One row per completed customer order across all channels.
resource: https://console.cloud.google.com/bigquery?p=acme&d=sales&t=orders
tags: [sales, orders, revenue]
timestamp: 2026-05-28T14:30:00Z
---

# Schema

| Column        | Type      | Description                              |
|---------------|-----------|------------------------------------------|
| `order_id`    | STRING    | Globally unique order identifier.        |
| `customer_id` | STRING    | Foreign key into [customers](/tables/customers.md). |
| `total_usd`   | NUMERIC   | Order total in US dollars.               |
| `placed_at`   | TIMESTAMP | When the customer submitted the order.   |

# Citations

[1] [BigQuery table schema](https://console.cloud.google.com/bigquery?p=acme&d=sales&t=orders)
```

### Example: Concept not bound to a resource

```markdown
---
type: Playbook
title: Incident response — data freshness alert
description: Steps to triage a freshness alert on the orders pipeline.
tags: [oncall, incident]
timestamp: 2026-04-12T09:00:00Z
---

# Trigger

A freshness alert fires when `orders` lags more than 30 minutes behind
its expected SLA. See the [orders table](/tables/orders.md).

# Steps

1. Check the [ingestion job dashboard](https://example.com/dash).
2. …
```

## Cross-linking

Two forms:

### Absolute (bundle-relative) links

Begin with `/`, interpreted relative to bundle root. Recommended — stable when documents are moved.

```markdown
See the [customers table](/tables/customers.md) for the join key.
```

### Relative links

Standard markdown relative paths.

```markdown
See the [neighboring concept](./other.md).
```

A link from concept A to concept B asserts a *relationship*. The specific kind (parent/child, references, joins-with, etc.) is conveyed by surrounding prose, not by the link itself. Consumers MUST tolerate broken links.

## Index Files

`index.md` may appear in any directory. Contains no frontmatter (except root index may declare `okf_version`). Body groups entries under headings:

```markdown
# Section / Group Heading

* [Title 1](relative-url-1) - short description of item 1
* [Title 2](relative-url-2) - short description of item 2

# Another Section

* [Subdirectory](subdir/) - short description of the subdirectory
```

## Log Files

`log.md` may appear at any level. Flat list of date-grouped entries, newest first:

```markdown
## 2026-05-22
* **Update**: Added new BigQuery table reference for [Customer Metrics](/tables/customer-metrics.md).
* **Creation**: Established the [Dataplex Playbook](/playbooks/dataplex.md).

## 2026-05-15
* **Initialization**: Created foundational directory structure.
```

Date headings MUST use ISO 8601 `YYYY-MM-DD` form. Bold prefix (`**Update**`, `**Creation**`) is convention, not requirement.

## Citations

Under `# Citations` heading at bottom of document, numbered:

```markdown
# Citations

[1] [BigQuery public dataset announcement](https://cloud.google.com/blog/products/data-analytics/...)
[2] [Internal data quality runbook](https://wiki.acme.internal/data/quality)
```

Citations MAY be absolute URLs, bundle-relative paths, or paths into a `references/` subdirectory.

## Conformance

A bundle is **conformant** with OKF v0.1 if:

1. Every non-reserved `.md` file has parseable YAML frontmatter
2. Every frontmatter block has a non-empty `type` field
3. Every reserved filename (`index.md`, `log.md`) follows its defined structure when present

All other constraints are soft guidance. Consumers MUST NOT reject a bundle because of:
- Missing optional frontmatter fields
- Unknown `type` values
- Unknown additional frontmatter keys
- Broken cross-links
- Missing `index.md` files

## Versioning

Bundles MAY declare OKF version via `okf_version: "0.1"` in bundle-root `index.md` frontmatter (the only place frontmatter is permitted in an `index.md`). Consumers that don't understand the version SHOULD attempt best-effort consumption.

## Example Bundle

```
my_bundle/
├── index.md
├── datasets/
│   ├── index.md
│   └── sales.md
└── tables/
    ├── index.md
    ├── orders.md
    └── customers.md
```
