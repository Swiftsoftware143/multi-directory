#!/bin/bash
BASE="http://127.0.0.1:3001/api/v1/email"
# Use the first template_id from list
TID=$(curl -s "$BASE/templates" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d[0]['id'] if d else 'none')")
echo "Using template: $TID"

echo ""
echo "=== Create Campaign ==="
curl -s -X POST "$BASE/campaigns" -H "Content-Type: application/json" \
  -d "{\"name\":\"May Newsletter\",\"template_id\":\"$TID\",\"status\":\"draft\"}"
echo ""
echo "=== List Campaigns ==="
curl -s "$BASE/campaigns"
echo ""
echo "=== Send Campaign ==="
CID=$(curl -s "$BASE/campaigns" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d[0]['id'] if d else 'none')")
echo "Using campaign: $CID"
curl -s -X POST "$BASE/campaigns/$CID/send"
echo ""