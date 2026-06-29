#!/bin/bash
BASE="http://127.0.0.1:3001/api/v1/email"
echo "=== Create Template ==="
RESP=$(curl -s -X POST "$BASE/templates" -H "Content-Type: application/json" -d '{"name":"Welcome Template","subject":"Welcome!","body":"<h1>Welcome</h1>","variables":["name"],"category":"general"}')
echo "$RESP"
echo ""

echo "=== List Templates ==="
curl -s "$BASE/templates"
echo ""

echo "=== Create Campaign ==="
curl -s -X POST "$BASE/campaigns" -H "Content-Type: application/json" -d '{"name":"May Newsletter","template_id":"00000000-0000-0000-0000-000000000000","status":"draft"}'
echo ""

echo "=== List Campaigns ==="
curl -s "$BASE/campaigns"
echo ""