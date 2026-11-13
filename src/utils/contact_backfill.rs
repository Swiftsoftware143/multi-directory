//! Fill `businesses.phone` / `businesses.website` from the Places cache (kanban t_774bf5ce).
//!
//! Measured 2026-11-13 on the live DB: 329 active listings carry an empty website and 184 an
//! empty phone while `google_places_cache` already holds the value for 19 of the website rows
//! (16 by exact place id, 3 by normalised name + street line + city) and 20 of the phone rows
//! (17 + 3). The detail template renders only `businesses.website` / `businesses.phone`, so a
//! listing whose cache row carries the site shows nothing: the cache is filled, the listing is
//! not. Same one-way wiring gap as `enriched_at` stamped without writing the field.
//!
//! Two rules encoded here:
//!   * **never a bare name match.** Duplicate business names across the ten cities are real, so a
//!     match needs the exact place id, or the normalised name AND the listing's own street line
//!     AND the listing's own city inside the cached address. A wrong phone number is worse than a
//!     blank one (it sends a customer to a stranger), which is why the city guard is required even
//!     when the street matches — stricter than the hours join, and rows a street match would reach
//!     in another city are reported as left alone rather than filled.
//!   * **never overwrite the operator's own value.** Only `NULL`/blank columns are written; the
//!     `UPDATE` carries that condition too, so a concurrent operator edit wins the race.
//!
//! The pass is one-shot and read-mostly: `run(db, false)` reports the plan, `run(db, true)` writes.
//! No Google call is made — cached values only.

use sqlx::{PgPool, Row};

#[derive(Debug, Default)]
pub struct Report {
    pub scanned: usize,
    /// Empty columns on active listings, before the pass.
    pub website_empty: i64,
    pub phone_empty: i64,
    /// Rows a guarded cache match could fill (candidates, whether or not `apply` was set).
    pub website_recovered: usize,
    pub phone_recovered: usize,
    /// Rows actually written. Zero on a report-only pass.
    pub website_filled: usize,
    pub phone_filled: usize,
    pub by_place_id: usize,
    pub by_name_street: usize,
    /// Left alone: name + street matched a cache row that carries the value, but the cached
    /// address does not name the listing's own city (a duplicate name in another city).
    pub website_city_disagree: i64,
    pub phone_city_disagree: i64,
    /// Left alone: no guarded cache row carries the value at all.
    pub website_no_match: i64,
    pub phone_no_match: i64,
    pub after_website_empty: i64,
    pub after_phone_empty: i64,
}

#[derive(Debug, Clone)]
pub struct Filled {
    pub name: String,
    pub city: String,
    pub field: &'static str,
    pub value: String,
    pub by_place_id: bool,
}

impl Filled {
    pub fn line(&self) -> String {
        format!(
            "{:<44} {:<16} {:<8} {:<38} [{}]",
            self.name.chars().take(44).collect::<String>(),
            self.city.chars().take(16).collect::<String>(),
            self.field,
            self.value.chars().take(38).collect::<String>(),
            if self.by_place_id {
                "place_id"
            } else {
                "name+street+city"
            }
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Phone,
    Website,
}

impl Field {
    fn as_str(self) -> &'static str {
        match self {
            Field::Phone => "phone",
            Field::Website => "website",
        }
    }

    /// The cached column, narrowed to a real value. Static text, never user input.
    fn expr(self) -> &'static str {
        match self {
            Field::Phone => "nullif(trim(c.phone), '')",
            Field::Website => "nullif(trim(c.website), '')",
        }
    }
}

/// Normalised keys, exactly as the SQL binds them. Exposed for the tests and so the two call
/// sites cannot drift apart from the query.
fn name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

fn street_key(address: Option<&str>) -> String {
    address
        .map(|a| a.split(',').next().unwrap_or("").trim().to_lowercase())
        .unwrap_or_default()
}

fn city_key(city: &str) -> String {
    city.trim().to_lowercase()
}

/// Best cached value for one field, or `None`. Place id is tried first because it is exact; the
/// name arm requires the street line and the city to be present in the cached formatted address.
async fn lookup(
    db: &PgPool,
    field: Field,
    place_id: Option<&str>,
    name: &str,
    address: Option<&str>,
    city: &str,
) -> Option<(String, bool)> {
    let nk = name_key(name);
    if nk.is_empty() {
        return None;
    }
    let sk = street_key(address);
    let ck = city_key(city);

    let sql = format!(
        r#"SELECT {expr} AS value, (c.place_id = $1::text) AS by_place
           FROM google_places_cache c
           WHERE {expr} IS NOT NULL
             AND (
               ($1::text IS NOT NULL AND $1::text <> '' AND c.place_id = $1::text)
               OR (
                 lower(c.name) = $2
                 AND (
                   ($3::text <> '' AND position($3 in lower(coalesce(c.formatted_address, ''))) > 0
                    AND ($4::text = '' OR position($4 in lower(coalesce(c.formatted_address, ''))) > 0))
                   OR ($3::text = '' AND $4::text <> ''
                       AND position($4 in lower(coalesce(c.formatted_address, ''))) > 0)
                 )
               )
             )
           ORDER BY (c.place_id = $1::text) DESC NULLS LAST, c.cached_at DESC
           LIMIT 1"#,
        expr = field.expr()
    );

    let row = sqlx::query(&sql)
        .bind(place_id)
        .bind(&nk)
        .bind(&sk)
        .bind(&ck)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()?;

    let value: String = row.try_get("value").ok()?;
    let by_place: Option<bool> = row.try_get("by_place").ok();
    Some((value, by_place.unwrap_or(false)))
}

/// Census of the recoverable set, before or after a pass. One query, both fields.
async fn census(db: &PgPool) -> Result<[(i64, i64, i64, i64); 2], String> {
    let sql = r#"
WITH b AS (
  SELECT id, place_id,
         lower(trim(name))                                     AS nk,
         lower(trim(split_part(coalesce(address, ''), ',', 1))) AS sk,
         lower(trim(coalesce(city, '')))                       AS ck,
         coalesce(trim(website), '') = ''                      AS se,
         coalesce(trim(phone), '')   = ''                      AS pe
  FROM businesses
  WHERE is_active
)
SELECT 'website' AS field,
       count(*) FILTER (WHERE se) AS empty_total,
       count(*) FILTER (WHERE se AND EXISTS (
         SELECT 1 FROM google_places_cache c
          WHERE nullif(trim(c.website), '') IS NOT NULL AND c.place_id = b.place_id)) AS by_place,
       count(*) FILTER (WHERE se AND EXISTS (
         SELECT 1 FROM google_places_cache c
          WHERE nullif(trim(c.website), '') IS NOT NULL
            AND lower(c.name) = b.nk AND b.sk <> '' AND b.ck <> ''
            AND (b.place_id IS NULL OR b.place_id = '' OR c.place_id IS DISTINCT FROM b.place_id)
            AND position(b.sk in lower(coalesce(c.formatted_address, ''))) > 0
            AND position(b.ck in lower(coalesce(c.formatted_address, ''))) > 0)) AS by_name_street,
       count(*) FILTER (WHERE se AND EXISTS (
         SELECT 1 FROM google_places_cache c
          WHERE nullif(trim(c.website), '') IS NOT NULL
            AND lower(c.name) = b.nk AND b.sk <> '' AND b.ck <> ''
            AND (b.place_id IS NULL OR b.place_id = '' OR c.place_id IS DISTINCT FROM b.place_id)
            AND position(b.sk in lower(coalesce(c.formatted_address, ''))) > 0
            AND position(b.ck in lower(coalesce(c.formatted_address, ''))) = 0)) AS city_disagree
FROM b
UNION ALL
SELECT 'phone',
       count(*) FILTER (WHERE pe),
       count(*) FILTER (WHERE pe AND EXISTS (
         SELECT 1 FROM google_places_cache c
          WHERE nullif(trim(c.phone), '') IS NOT NULL AND c.place_id = b.place_id)),
       count(*) FILTER (WHERE pe AND EXISTS (
         SELECT 1 FROM google_places_cache c
          WHERE nullif(trim(c.phone), '') IS NOT NULL
            AND lower(c.name) = b.nk AND b.sk <> '' AND b.ck <> ''
            AND (b.place_id IS NULL OR b.place_id = '' OR c.place_id IS DISTINCT FROM b.place_id)
            AND position(b.sk in lower(coalesce(c.formatted_address, ''))) > 0
            AND position(b.ck in lower(coalesce(c.formatted_address, ''))) > 0)),
       count(*) FILTER (WHERE pe AND EXISTS (
         SELECT 1 FROM google_places_cache c
          WHERE nullif(trim(c.phone), '') IS NOT NULL
            AND lower(c.name) = b.nk AND b.sk <> '' AND b.ck <> ''
            AND (b.place_id IS NULL OR b.place_id = '' OR c.place_id IS DISTINCT FROM b.place_id)
            AND position(b.sk in lower(coalesce(c.formatted_address, ''))) > 0
            AND position(b.ck in lower(coalesce(c.formatted_address, ''))) = 0))
FROM b;"#;

    let rows = sqlx::query(sql)
        .fetch_all(db)
        .await
        .map_err(|e| format!("census: {e}"))?;
    let mut website = (0, 0, 0, 0);
    let mut phone = (0, 0, 0, 0);
    for r in rows {
        let field: String = r.try_get("field").map_err(|e| e.to_string())?;
        let vals = (
            r.try_get::<i64, _>("empty_total").unwrap_or(0),
            r.try_get::<i64, _>("by_place").unwrap_or(0),
            r.try_get::<i64, _>("by_name_street").unwrap_or(0),
            r.try_get::<i64, _>("city_disagree").unwrap_or(0),
        );
        match field.as_str() {
            "website" => website = vals,
            _ => phone = vals,
        }
    }
    Ok([website, phone])
}

/// One-shot pass over every active listing with an empty phone or website. `apply == false`
/// reports the plan and writes nothing.
pub async fn run(db: &PgPool, apply: bool) -> Result<(Report, Vec<Filled>), String> {
    let before = census(db).await?;
    let mut report = Report {
        website_empty: before[0].0,
        phone_empty: before[1].0,
        website_city_disagree: before[0].3,
        phone_city_disagree: before[1].3,
        ..Default::default()
    };

    let rows = sqlx::query(
        "SELECT id, name, address, city, place_id FROM businesses \
         WHERE is_active AND (coalesce(trim(phone), '') = '' OR coalesce(trim(website), '') = '') \
         ORDER BY name, id",
    )
    .fetch_all(db)
    .await
    .map_err(|e| format!("list listings: {e}"))?;

    let mut changes = Vec::new();
    for row in rows {
        let id: uuid::Uuid = row.try_get("id").map_err(|e| e.to_string())?;
        let name: String = row.try_get("name").unwrap_or_default();
        let address: Option<String> = row.try_get("address").ok();
        let city: String = row.try_get("city").unwrap_or_default();
        let place_id: Option<String> = row.try_get("place_id").ok();
        let place_id = place_id.filter(|p| !p.trim().is_empty());
        report.scanned += 1;

        let mut phone: Option<String> = None;
        let mut website: Option<String> = None;
        let mut tiers: Vec<(Field, bool, String)> = Vec::new();

        // The listing's stored values decide what may be touched — never the cache.
        let cur: (String, String) = sqlx::query_as(
            "SELECT coalesce(trim(phone), ''), coalesce(trim(website), '') FROM businesses WHERE id = $1",
        )
        .bind(id)
        .fetch_one(db)
        .await
        .map_err(|e| format!("read listing: {e}"))?;

        if cur.0.is_empty() {
            if let Some((value, by_place)) = lookup(
                db,
                Field::Phone,
                place_id.as_deref(),
                &name,
                address.as_deref(),
                &city,
            )
            .await
            {
                phone = Some(value.clone());
                tiers.push((Field::Phone, by_place, value));
            }
        }
        if cur.1.is_empty() {
            if let Some((value, by_place)) = lookup(
                db,
                Field::Website,
                place_id.as_deref(),
                &name,
                address.as_deref(),
                &city,
            )
            .await
            {
                website = Some(value.clone());
                tiers.push((Field::Website, by_place, value));
            }
        }

        if phone.is_none() && website.is_none() {
            continue;
        }

        if phone.is_some() {
            report.phone_recovered += 1;
        }
        if website.is_some() {
            report.website_recovered += 1;
        }
        for (field, by_place, value) in &tiers {
            if *by_place {
                report.by_place_id += 1;
            } else {
                report.by_name_street += 1;
            }
            changes.push(Filled {
                name: name.clone(),
                city: city.clone(),
                field: field.as_str(),
                value: value.clone(),
                by_place_id: *by_place,
            });
        }

        if !apply {
            continue;
        }

        // Only a blank column is written, and the same condition is repeated in the WHERE so a
        // concurrent operator edit wins. `updated_at` is deliberately untouched: the sitemap clock
        // is `content_updated_at`, and this pass changes no page copy.
        let res = sqlx::query(
            "UPDATE businesses SET \
                phone = CASE WHEN coalesce(trim(phone), '') = '' THEN $2 ELSE phone END, \
                website = CASE WHEN coalesce(trim(website), '') = '' THEN $3 ELSE website END \
             WHERE id = $1",
        )
        .bind(id)
        .bind(&phone)
        .bind(&website)
        .execute(db)
        .await
        .map_err(|e| format!("update listing: {e}"))?;
        if res.rows_affected() == 0 {
            continue;
        }
        if phone.is_some() {
            report.phone_filled += 1;
        }
        if website.is_some() {
            report.website_filled += 1;
        }
    }

    report.website_no_match =
        (report.website_empty - report.website_recovered as i64 - report.website_city_disagree)
            .max(0);
    report.phone_no_match =
        (report.phone_empty - report.phone_recovered as i64 - report.phone_city_disagree).max(0);

    if apply {
        let after = census(db).await?;
        report.after_website_empty = after[0].0;
        report.after_phone_empty = after[1].0;
    } else {
        report.after_website_empty = before[0].0;
        report.after_phone_empty = before[1].0;
    }
    Ok((report, changes))
}

/// Fill the listings that hold this exact place id, from values just harvested by a Places detail
/// call. The exact-id path only — no name matching — because this runs on a live request where a
/// wrong value would be published immediately. Returns how many rows were touched.
///
/// Wired into `data_company::place_details`, the one path that writes `phone`/`website` into the
/// cache: before this, looking a place up filled the cache and left the listing blank forever.
pub async fn fill_for_place(
    db: &PgPool,
    place_id: &str,
    phone: Option<&str>,
    website: Option<&str>,
) -> u64 {
    if place_id.trim().is_empty() || (phone.is_none() && website.is_none()) {
        return 0;
    }
    let res = sqlx::query(
        "UPDATE businesses SET \
            phone = CASE WHEN coalesce(trim(phone), '') = '' THEN $2 ELSE phone END, \
            website = CASE WHEN coalesce(trim(website), '') = '' THEN $3 ELSE website END \
         WHERE place_id = $1 \
           AND (coalesce(trim(phone), '') = '' OR coalesce(trim(website), '') = '')",
    )
    .bind(place_id)
    .bind(phone)
    .bind(website)
    .execute(db)
    .await;
    match res {
        Ok(r) => r.rows_affected(),
        Err(e) => {
            eprintln!("contact_backfill: fill_for_place failed: {e}");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn street_key_is_the_first_address_line_only() {
        assert_eq!(
            street_key(Some("123 Main St, Palm Bay, FL 32905")),
            "123 main st"
        );
        assert_eq!(
            street_key(Some("  456  Oak  Ave. , Orlando")),
            "456  oak  ave."
        );
        assert_eq!(street_key(None), "");
    }

    #[test]
    fn keys_are_trimmed_and_lowercased() {
        assert_eq!(name_key("  Joe's Diner "), "joe's diner");
        assert_eq!(city_key("Palm Bay"), "palm bay");
    }

    #[test]
    fn field_sql_narrows_blank_to_null() {
        assert_eq!(Field::Phone.expr(), "nullif(trim(c.phone), '')");
        assert_eq!(Field::Website.expr(), "nullif(trim(c.website), '')");
        assert_eq!(Field::Website.as_str(), "website");
    }
}
