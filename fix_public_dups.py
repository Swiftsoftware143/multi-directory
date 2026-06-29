p = "/opt/swift/multidirectory-rust/src/handlers/public.rs"
c = open(p).read()
# Remove the duplicate import block after the pub use re-exports
old = """// ── Homepage / Directory / Business data endpoints ──────────────
use axum::{
    extract::{Path, State, Query},
    Json,
};
use serde_json::{json, Value};
use crate::{AppState, error::ApiResult};

"""
new = """// ── Homepage / Directory / Business data endpoints ──────────────

"""
if old in c:
    c = c.replace(old, new)
    open(p, "w").write(c)
    print("Removed duplicate imports")
else:
    print("Pattern not found")
    # Print the area around it
    lines = c.split("\n")
    for i, line in enumerate(lines):
        if "Homepage / Directory / Business data" in line:
            for j in range(max(0,i-1), min(len(lines), i+8)):
                print(f"{j+1}: {lines[j]}")