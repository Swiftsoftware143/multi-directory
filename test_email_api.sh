curl -s -X POST http://127.0.0.1:3001/api/v1/email/templates \
  -H 'Content-Type: application/json' \
  -d '{"name":"Test Template","subject":"Hello {{name}}","body":"<p>Hi {{name}}, welcome!</p>","category":"general","variables":["name"]}'
echo ""
curl -s http://127.0.0.1:3001/api/v1/email/templates
echo ""