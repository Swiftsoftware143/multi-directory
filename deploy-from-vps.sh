#!/bin/bash
# Auto-deploy: pull latest from GitHub and rebuild
set -e

cd /opt/swift/multidirectory-rust

# Acquire build lock — one at a time (prevents OOM)
for i in $(seq 1 60); do
  if mkdir /tmp/rust-build.lock 2>/dev/null; then
    break
  fi
  sleep 2
  if [ "$i" -eq 60 ]; then
    echo "ERROR: Could not acquire build lock after 120s"
    exit 1
  fi
done
trap 'rmdir /tmp/rust-build.lock 2>/dev/null' EXIT

# Pull latest
git pull origin main

# Build
CARGO_BUILD_JOBS=1 /root/.cargo/bin/cargo build --release

# Restart
systemctl restart multidirectory
sleep 1
systemctl --no-pager status multidirectory --no-pager | head -10
echo "=== Deploy complete ==="
