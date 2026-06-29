p = "/opt/swift/multidirectory-rust/src/handlers/public.rs"
c = open(p).read()

# Add the missing landing_page and theme functions
missing = """

// ── Landing Pages (re-exported from public_pages) ────────────────
pub use crate::handlers::public_pages::{
    list_landing_pages,
    create_landing_page,
    get_landing_page,
    update_landing_page,
    delete_landing_page,
    toggle_publish,
    list_public_themes,
    create_public_theme,
    get_public_theme,
    update_public_theme,
    delete_public_theme,
};

// ── Homepage / Directory / Business data endpoints ──────────────
use axum::{
    extract::{Path, State, Query},
    Json,
};
use serde_json::{json, Value};
use crate::{AppState, error::ApiResult};

pub async fn homepage_data(
    State(state): State<AppState>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"status": "ok", "message": "Homepage data endpoint"})))
}

pub async fn directory_data(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"slug": slug, "message": "Directory data endpoint"})))
}

pub async fn business_data(
    State(state): State<AppState>,
    Path((slug, business_id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"slug": slug, "business_id": business_id, "message": "Business data endpoint"})))
}
"""

# Insert before the last line (closing)
lines = c.rstrip().split("\n")
insert_pos = len(lines)
# Find where to insert - before the last function or at end
for i in range(len(lines)-1, -1, -1):
    line = lines[i].strip()
    if line.startswith("use ") or line.startswith("//") or line == "":
        continue
    if line.startswith("pub") or line.startswith("}") or line.startswith("#"):
        insert_pos = i
        break

lines.insert(insert_pos, missing)
c = "\n".join(lines)
open(p, "w").write(c)
print("Added missing functions to public.rs")