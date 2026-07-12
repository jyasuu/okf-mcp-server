---
name: test-okf-tools
description: Run end-to-end tests of the OKF MCP server tools. Use when testing MCP tools, verifying write/read/search/delete, validating data, or benchmarking with large datasets. Trigger on "test okf", "mcp test", "e2e test", "benchmark search".
---

# Test OKF MCP Tools

Run end-to-end tests of the OKF MCP server via raw MCP protocol over stdio.

## Prerequisites

Build the server first:

```
cargo build
```

## Quick Test

Run a full tool test suite:

```bash
RUST_LOG=error target/debug/okf-mcp-server < /tmp/okf_test.txt
```

Generate test data with the script below, then pipe it to the server.

## Test Script Template

Save as `/tmp/okf_test.sh`:

```bash
#!/bin/bash
set -e

send() {
  echo "$1"
  sleep 0.2
}

# Initialize
send '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0.1"}}}'
send '{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}'

# Write concepts
send '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"okf_write_concept","arguments":{"bundle":"default","concept_id":"tables/orders","data":"{\"type\":\"Table\",\"title\":\"Orders\",\"description\":\"Customer orders\",\"tags\":[\"billing\"],\"body\":\"Order data.\"}"}}}'

send '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"okf_write_concept","arguments":{"bundle":"default","concept_id":"tables/customers","data":"{\"type\":\"Table\",\"title\":\"Customers\",\"tags\":[\"crm\"],\"body\":\"Customer records.\"}"}}}'

send '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"okf_write_concept","arguments":{"bundle":"default","concept_id":"views/revenue","data":"{\"type\":\"View\",\"title\":\"Revenue\",\"tags\":[\"finance\"],\"body\":\"Revenue analysis.\"}"}}}'

# Read
send '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"okf_read_concept","arguments":{"bundle":"default","concept_id":"tables/orders"}}}'

# Search
send '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"okf_search","arguments":{"bundle":"default","query":"order"}}}'

# List
send '{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"okf_list_concepts","arguments":{"bundle":"default"}}}'

# Validate
send '{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"okf_validate_bundle","arguments":{"bundle":"default"}}}'

# Backlinks
send '{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"okf_get_backlinks","arguments":{"bundle":"default","concept_id":"tables/orders"}}}'

# Delete
send '{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"okf_delete_concept","arguments":{"bundle":"default","concept_id":"tables/customers"}}}'

# Final list
send '{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"okf_list_concepts","arguments":{"bundle":"default"}}}'
```

## Validation Test

Test that invalid data is rejected:

```bash
# Missing type
send '{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"okf_write_concept","arguments":{"bundle":"default","concept_id":"bad","data":"{\"title\":\"No type\"}"}}}'

# Invalid mode
send '{"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"okf_write_concept","arguments":{"bundle":"default","concept_id":"bad","data":"{\"type\":\"Table\",\"mode\":\"invalid\"}"}}}'

# Invalid tags
send '{"jsonrpc":"2.0","id":22,"method":"tools/call","params":{"name":"okf_write_concept","arguments":{"bundle":"default","concept_id":"bad","data":"{\"type\":\"Table\",\"tags\":[123]}"}}}'

# Invalid title type
send '{"jsonrpc":"2.0","id":23,"method":"tools/call","params":{"name":"okf_write_concept","arguments":{"bundle":"default","concept_id":"bad","data":"{\"type\":\"Table\",\"title\":42}"}}}'
```

## Large Dataset Benchmark

Generate 1000 concepts and test search performance:

```python
import json, subprocess, time

concepts = []
for i in range(1000):
    concepts.append({
        'bundle': 'default',
        'concept_id': f'tables/record_{i:04d}',
        'data': json.dumps({
            'type': 'Table',
            'title': f'Record {i}',
            'description': f'Description for record {i}',
            'tags': ['dataset', f'group_{i % 10}', f'category_{i % 5}'],
            'body': f'## Overview\n\nRecord {i} about topic {(i * 7) % 100}.'
        })
    })

# Build JSON-RPC messages
msgs = [
    json.dumps({'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2024-11-05','capabilities':{},'clientInfo':{'name':'test','version':'0.1'}}}),
    json.dumps({'jsonrpc':'2.0','method':'notifications/initialized','params':{}}),
]
for i, c in enumerate(concepts):
    msgs.append(json.dumps({'jsonrpc':'2.0','id':i+2,'method':'tools/call','params':{'name':'okf_write_concept','arguments':c}}))

# Search queries
for q in ['record 42', 'topic 50', 'batch 10 relevance']:
    msgs.append(json.dumps({'jsonrpc':'2.0','id':2000,'method':'tools/call','params':{'name':'okf_search','arguments':{'bundle':'default','query':q}}}))

input_text = '\n'.join(msgs)
proc = subprocess.run(
    ['target/debug/okf-mcp-server'],
    input=input_text, capture_output=True, text=True, timeout=30
)

# Parse results
for line in proc.stdout.strip().split('\n'):
    try:
        msg = json.loads(line)
        mid = msg.get('id', 0)
        if mid >= 2000:
            text = msg['result']['content'][0]['text']
            data = json.loads(text)
            print(f'Query id={mid}: {len(data)} results')
    except:
        pass
```

## Expected Results

| Tool | Expected |
|------|----------|
| `write_concept` (valid) | Success with concept JSON |
| `write_concept` (missing type) | Error: `data.type is required` |
| `write_concept` (bad mode) | Error: `data.mode must be 'create', 'update', or 'upsert'` |
| `read_concept` | Full concept with frontmatter + body |
| `search` (query) | Ranked results with scores and snippets |
| `list_concepts` | Array of concept IDs |
| `list_concepts` (prefix) | Filtered by path prefix |
| `validate_bundle` | `{errors: [], warnings: [...]}` |
| `get_backlinks` | Array of concept IDs linking to target |
| `delete_concept` | `{deleted: true}` |

## Cleanup

After testing, remove generated data:

```bash
rm -rf bundles/
```
