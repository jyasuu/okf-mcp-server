#!/bin/bash
set -e

# OKF MCP Server E2E Test
# Usage: ./tests/test_mcp.sh

send() {
  echo "$1"
  sleep 0.15
}

run_test() {
  local name="$1"
  local expected="$2"
  local actual="$3"
  if echo "$actual" | grep -q "$expected"; then
    echo "PASS: $name"
  else
    echo "FAIL: $name (expected '$expected' in: $actual)"
  fi
}

echo "=== OKF MCP Server E2E Tests ==="
echo ""

RESULTS=$( (
  # Initialize
  send '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0.1"}}}'
  send '{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}'

  # 1. List bundles
  send '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"okf_list_bundles","arguments":{"bundle":""}}}'

  # 2. Write valid concept
  send '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"okf_write_concept","arguments":{"bundle":"default","concept_id":"tables/orders","data":"{\"type\":\"Table\",\"title\":\"Orders\",\"description\":\"Customer orders\",\"tags\":[\"billing\",\"sales\"],\"body\":\"## Overview\\n\\nOrder data.\"}"}}}'

  # 3. Write second concept
  send '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"okf_write_concept","arguments":{"bundle":"default","concept_id":"tables/customers","data":"{\"type\":\"Table\",\"title\":\"Customers\",\"tags\":[\"crm\"],\"body\":\"Customer records.\"}"}}}'

  # 4. Write View
  send '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"okf_write_concept","arguments":{"bundle":"default","concept_id":"views/revenue","data":"{\"type\":\"View\",\"title\":\"Revenue\",\"tags\":[\"finance\"],\"body\":\"Revenue analysis.\"}"}}}'

  # 5. Validation: missing type
  send '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"okf_write_concept","arguments":{"bundle":"default","concept_id":"bad","data":"{\"title\":\"No type\"}"}}}'

  # 6. Validation: bad mode
  send '{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"okf_write_concept","arguments":{"bundle":"default","concept_id":"bad","data":"{\"type\":\"Table\",\"mode\":\"nope\"}"}}}'

  # 7. Validation: bad tags
  send '{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"okf_write_concept","arguments":{"bundle":"default","concept_id":"bad","data":"{\"type\":\"Table\",\"tags\":[123]}"}}}'

  # 8. Read concept
  send '{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"okf_read_concept","arguments":{"bundle":"default","concept_id":"tables/orders"}}}'

  # 9. Search by query
  send '{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"okf_search","arguments":{"bundle":"default","query":"order"}}}'

  # 10. List all
  send '{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"okf_list_concepts","arguments":{"bundle":"default"}}}'

  # 11. List by prefix
  send '{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"okf_list_concepts","arguments":{"bundle":"default","prefix":"tables/"}}}'

  # 12. List by tag
  send '{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"okf_list_concepts","arguments":{"bundle":"default","tag":"finance"}}}'

  # 13. Validate bundle
  send '{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"okf_validate_bundle","arguments":{"bundle":"default"}}}'

  # 14. Get backlinks
  send '{"jsonrpc":"2.0","id":15,"method":"tools/call","params":{"name":"okf_get_backlinks","arguments":{"bundle":"default","concept_id":"tables/orders"}}}'

  # 15. Delete concept
  send '{"jsonrpc":"2.0","id":16,"method":"tools/call","params":{"name":"okf_delete_concept","arguments":{"bundle":"default","concept_id":"tables/customers"}}}'

  # 16. Final list
  send '{"jsonrpc":"2.0","id":17,"method":"tools/call","params":{"name":"okf_list_concepts","arguments":{"bundle":"default"}}}'

) | RUST_LOG=error target/debug/okf-mcp-server 2>/dev/null )

# Parse results
echo "$RESULTS" | python3 -c "
import sys, json

results = {}
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try:
        msg = json.loads(line)
    except:
        continue
    mid = msg.get('id')
    if not mid: continue
    if msg.get('error'):
        results[mid] = {'error': msg['error']['message']}
    elif 'result' in msg:
        content = msg['result'].get('content', [{}])
        text = content[0].get('text', '') if content else ''
        try:
            results[mid] = {'data': json.loads(text)}
        except:
            results[mid] = {'text': text}

# Tests
passed = 0
failed = 0

def check(name, ok):
    global passed, failed
    if ok:
        print(f'  PASS: {name}')
        passed += 1
    else:
        print(f'  FAIL: {name}')
        failed += 1

print('--- Write Tests ---')
check('write valid concept', results.get(3, {}).get('data', {}).get('id') == 'tables/orders')
check('write second concept', results.get(4, {}).get('data', {}).get('id') == 'tables/customers')
check('write view', results.get(5, {}).get('data', {}).get('id') == 'views/revenue')
check('reject missing type', 'data.type is required' in results.get(6, {}).get('error', ''))
check('reject bad mode', 'data.mode must be' in results.get(7, {}).get('error', ''))
check('reject bad tags', 'tags array elements must be strings' in results.get(8, {}).get('error', ''))

print('--- Read Tests ---')
read_data = results.get(9, {}).get('data', {})
check('read concept', read_data.get('id') == 'tables/orders')
check('read has body', 'Overview' in read_data.get('body', ''))
check('read has tags', 'billing' in str(read_data.get('frontmatter', {}).get('tags', '')))

print('--- Search Tests ---')
search_results = results.get(10, {}).get('data', [])
check('search finds orders', len(search_results) > 0 and search_results[0].get('concept_id') == 'tables/orders')

print('--- List Tests ---')
all_concepts = results.get(11, {}).get('data', [])
check('list all has 3 concepts', len(all_concepts) == 3)
tables_only = results.get(12, {}).get('data', [])
check('list prefix tables/ has 2', len(tables_only) == 2)
finance_only = results.get(13, {}).get('data', [])
check('list tag finance has 1', len(finance_only) == 1 and finance_only[0] == 'views/revenue')

print('--- Validate Tests ---')
validate = results.get(14, {}).get('data', {})
check('validate no errors', len(validate.get('errors', [])) == 0)
check('validate has warnings', len(validate.get('warnings', [])) > 0)

print('--- Backlinks Tests ---')
check('backlinks empty (no links)', results.get(15, {}).get('data', []) == [])

print('--- Delete Tests ---')
check('delete returns true', results.get(16, {}).get('data', {}).get('deleted') == True)
after_delete = results.get(17, {}).get('data', [])
check('list after delete has 2', len(after_delete) == 2)

print(f'')
print(f'Results: {passed} passed, {failed} failed out of {passed + failed}')
" 2>&1

# Cleanup
rm -rf bundles/
