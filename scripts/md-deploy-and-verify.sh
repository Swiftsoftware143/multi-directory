#!/usr/bin/env bash
# Multi-Directory — put the freshly built release binary live, and PROVE the running
# process is the artefact on disk (kanban t_f97664fe).
#
# Why a script: the binary is a BIND MOUNT of ONE FILE
#   /opt/swift/apps/multi-directory/target/release/multidirectory -> /app/multidirectory
# so a rebuild writes a NEW inode and the running container keeps the OLD one. The restart
# is what re-binds the mount. mtime is never evidence; the check is md5(host artefact) ==
# md5(/app/multidirectory) inside the container, plus a real HTTP answer from the api.
#
#   bash scripts/md-deploy-and-verify.sh            # deploy the built artefact
set -euo pipefail

APP=/opt/swift/apps/multi-directory
BIN="$APP/target/release/multidirectory"
C=multidirectory
API=http://127.0.0.1:8089/api/v1/health

[ -x "$BIN" ] || { echo "FAIL: no binary at $BIN"; exit 1; }
want=$(md5sum "$BIN" | cut -d' ' -f1)
before=$(docker exec "$C" md5sum /app/multidirectory | cut -d' ' -f1)
echo "artefact=$want  running-before=$before"

docker restart "$C" >/dev/null
for i in $(seq 1 24); do
    sleep 5
    got=$(docker exec "$C" md5sum /app/multidirectory 2>/dev/null | cut -d' ' -f1 || true)
    health=$(docker inspect "$C" --format '{{.State.Health.Status}}' 2>/dev/null || echo unknown)
    code=$(curl -s -o /dev/null -w '%{http_code}' -m 5 "$API" || true)
    if [ "$got" = "$want" ] && [ "$code" = "200" ]; then
        echo "PASS: running binary == artefact (md5 $want), health=$health api=$code (after ~$((i * 5))s)"
        exit 0
    fi
done
echo "FAIL: md5 host=$want running=$got health=$health api=$code"
exit 1
