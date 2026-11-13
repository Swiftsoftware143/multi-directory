//! One-shot re-file of listings whose own name and cached Places types contradict the category
//! they were published under (kanban t_b2f7201e).
//!
//! Measured 2026-11-13: "More Landscaping in Winter Garden" served a blow-dry bar, a gym and a
//! lounge, because the importer stamped the SEARCH TERM's category on every result and the guard
//! only vetoed a category the NAME contradicted — and none of those names name a trade at all.
//!
//! This pass is deterministic and reproducible: the shared `category_guard::verdict` rule, the
//! provider types already cached in `google_places_cache` (exact place id first, then the same
//! name in the same city), and a fixed family→category map. No model, no per-row judgement, no
//! guessing: a listing with no evidence or with conflicting evidence is left exactly where it is
//! and reported instead.

use crate::utils::category_guard::{self, Verdict};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, Default)]
pub struct Report {
    pub scanned: usize,
    pub supported: usize,
    pub unknown: usize,
    pub generic: usize,
    pub contradicted: usize,
    pub refiled: usize,
    pub unresolved: usize,
    pub copy_rewritten: usize,
}

#[derive(Debug, Clone)]
pub struct Change {
    pub name: String,
    pub from: String,
    pub to: String,
    pub evidence: Vec<&'static str>,
}

impl Change {
    pub fn line(&self) -> String {
        format!(
            "{:<26} -> {:<27} {:<46} [{}]",
            self.from,
            self.to,
            self.name.chars().take(46).collect::<String>(),
            self.evidence.join(",")
        )
    }
}

/// The provider types we may judge a listing by. Exact place id first; otherwise a cache row for
/// the same name whose formatted address names the listing's own city — a same-named business in
/// another city is a different business and must never supply evidence.
async fn place_types(db: &PgPool, place_id: &str, name: &str, city: &str) -> Vec<String> {
    if !place_id.is_empty() {
        if let Ok(Some(csv)) = sqlx::query_scalar::<_, String>(
            "SELECT array_to_string(types, ',') FROM google_places_cache \
             WHERE place_id = $1 AND types IS NOT NULL LIMIT 1",
        )
        .bind(place_id)
        .fetch_optional(db)
        .await
        {
            return split_csv(&csv);
        }
    }
    if city.trim().is_empty() {
        return Vec::new();
    }
    sqlx::query_scalar::<_, String>(
        "SELECT array_to_string(types, ',') FROM google_places_cache \
         WHERE lower(name) = lower($1) AND types IS NOT NULL \
           AND lower(coalesce(formatted_address, '')) LIKE '%' || lower($2) || '%' \
         ORDER BY cached_at DESC LIMIT 1",
    )
    .bind(name)
    .bind(city.trim())
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .map(|csv| split_csv(&csv))
    .unwrap_or_default()
}

fn split_csv(csv: &str) -> Vec<String> {
    csv.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// Walk every active listing. With `apply` false nothing is written — the caller prints the plan.
pub async fn run(db: &PgPool, apply: bool) -> Result<(Report, Vec<Change>), sqlx::Error> {
    let rows = sqlx::query(
        "SELECT b.id, b.directory_id, b.name, coalesce(b.place_id, '') AS place_id, \
                coalesce(dc.name, '') AS category, coalesce(d.city, '') AS dir_city \
         FROM businesses b \
         LEFT JOIN directory_categories dc ON dc.id = b.category_id \
         LEFT JOIN directories d ON d.id = b.directory_id \
         WHERE COALESCE(b.is_active, true) \
         ORDER BY b.name",
    )
    .fetch_all(db)
    .await?;

    let mut report = Report {
        scanned: rows.len(),
        ..Default::default()
    };
    let mut changes: Vec<Change> = Vec::new();
    let mut ids: Vec<(Uuid, Uuid, String)> = Vec::new(); // (business, directory, old category)

    for row in &rows {
        let id: Uuid = row.try_get("id")?;
        let name: String = row.try_get("name").unwrap_or_default();
        let place_id: String = row.try_get("place_id").unwrap_or_default();
        let category: String = row.try_get("category").unwrap_or_default();
        let dir_city: String = row.try_get("dir_city").unwrap_or_default();
        let directory_id: Option<Uuid> = row.try_get("directory_id").unwrap_or(None);

        // A generic bucket ("Other", "Professional") is an honest label and is never checked; a
        // listing with no category at all has nothing to contradict.
        if category_guard::category_family(&category).is_none() {
            report.generic += 1;
            continue;
        }
        let types = place_types(db, &place_id, &name, &dir_city).await;
        match category_guard::verdict(&name, &types, &category) {
            Verdict::Supported => report.supported += 1,
            Verdict::Unknown => report.unknown += 1,
            Verdict::Contradicted(families) => {
                report.contradicted += 1;
                match category_guard::target_category(&families) {
                    Some(target) if !target.eq_ignore_ascii_case(&category) => {
                        changes.push(Change {
                            name: name.clone(),
                            from: category.clone(),
                            to: target.to_string(),
                            evidence: families.clone(),
                        });
                        ids.push((id, directory_id.unwrap_or_default(), category.clone()));
                    }
                    _ => report.unresolved += 1, // evidence names a trade we hold no bucket for
                }
            }
        }
    }

    if !apply {
        return Ok((report, changes));
    }

    for (change, (id, directory_id, old_category)) in changes.iter().zip(ids.iter()) {
        let target_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM directory_categories \
             WHERE lower(name) = lower($1) AND (directory_id = $2 OR directory_id IS NULL) \
             ORDER BY directory_id NULLS LAST LIMIT 1",
        )
        .bind(&change.to)
        .bind(*directory_id)
        .fetch_optional(db)
        .await?;
        let Some(target_id) = target_id else {
            report.unresolved += 1;
            continue;
        };
        let updated = sqlx::query(
            "UPDATE businesses \
                SET category_id = $2, updated_at = NOW(), content_updated_at = NOW() \
              WHERE id = $1 AND COALESCE(is_active, true) \
                AND category_id IS DISTINCT FROM $2",
        )
        .bind(id)
        .bind(target_id)
        .execute(db)
        .await
        .map(|r| r.rows_affected())
        .unwrap_or(0);
        if updated == 0 {
            continue;
        }
        report.refiled += 1;
        // The paragraph this app wrote names the trade; re-render it, but only when the stored
        // text is provably ours under the OLD category.
        if crate::utils::business_copy::rewrite_for_category_change(db, *id, Some(old_category))
            .await
        {
            report.copy_rewritten += 1;
        }
    }

    Ok((report, changes))
}
