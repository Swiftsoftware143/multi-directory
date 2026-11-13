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

/// One cached provider record. Identity keys are computed once at load time so the four arms are
/// plain string comparisons and can be unit-tested without a database.
#[derive(Debug, Clone)]
struct CacheRow {
    place_id: String,
    name_key: String,
    address_lc: String,
    phone10: String,
    host: String,
    street3: String,
    cached_ts: f64,
    types: Vec<String>,
}

/// The whole cache, indexed for lookup. 4.5k rows: loading once per pass is cheaper and far more
/// testable than one SQL round trip per listing, and it makes the union-of-arms semantics explicit.
#[derive(Debug, Default)]
struct PlaceIndex {
    rows: Vec<CacheRow>,
    by_place: std::collections::HashMap<String, Vec<usize>>,
    by_name: std::collections::HashMap<String, Vec<usize>>,
}

impl PlaceIndex {
    async fn load(db: &PgPool) -> Result<Self, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT coalesce(place_id, '') AS place_id, coalesce(name, '') AS name, \
                    coalesce(formatted_address, '') AS formatted_address, \
                    coalesce(phone, '') AS phone, coalesce(website, '') AS website, \
                    array_to_string(coalesce(types, '{}'), ',') AS types, \
                    coalesce(extract(epoch FROM cached_at), 0)::float8 AS cached_ts \
             FROM google_places_cache",
        )
        .fetch_all(db)
        .await?;

        let mut rows_out: Vec<CacheRow> = Vec::with_capacity(rows.len());
        for row in rows {
            let place_id: String = row.try_get("place_id").unwrap_or_default();
            let name: String = row.try_get("name").unwrap_or_default();
            let address: String = row.try_get("formatted_address").unwrap_or_default();
            let phone: String = row.try_get("phone").unwrap_or_default();
            let website: String = row.try_get("website").unwrap_or_default();
            let csv: String = row.try_get("types").unwrap_or_default();
            let cached_ts: f64 = row.try_get("cached_ts").unwrap_or(0.0);
            rows_out.push(CacheRow {
                place_id,
                name_key: norm_name(&name),
                address_lc: address.to_lowercase(),
                phone10: digits_last10(&phone),
                host: website_host(&website),
                street3: street3_of_address(&address),
                cached_ts,
                types: split_csv(&csv),
            });
        }
        Ok(Self::build(rows_out))
    }

    /// Index a set of already-normalised rows. Shared by `load` and the tests so the two cannot
    /// disagree about what an arm means.
    fn build(rows: Vec<CacheRow>) -> Self {
        let mut index = PlaceIndex {
            rows,
            by_place: Default::default(),
            by_name: Default::default(),
        };
        for i in 0..index.rows.len() {
            let place_id = index.rows[i].place_id.clone();
            let name_key = index.rows[i].name_key.clone();
            if !place_id.is_empty() {
                index.by_place.entry(place_id).or_default().push(i);
            }
            if !name_key.is_empty() {
                index.by_name.entry(name_key).or_default().push(i);
            }
        }
        index
    }

    /// The cached records whose identity matches this listing — the union of every arm. Empty
    /// means we hold NO record for the business at all, which is a different reportable fact from
    /// "we hold a record and it names no trade".
    fn matched(
        &self,
        place_id: &str,
        name: &str,
        city: &str,
        phone: &str,
        website: &str,
        address: &str,
    ) -> std::collections::BTreeSet<usize> {
        let mut picked: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();

        // Arm 0: the exact place id. No name requirement — the id IS the identity.
        if !place_id.is_empty() {
            if let Some(ids) = self.by_place.get(place_id) {
                if let Some(i) = self.latest(ids, |_| true) {
                    picked.insert(i);
                }
            }
        }

        // Every other arm requires the same business name, punctuation-insensitively. Phone and
        // website alone are not evidence: 41 cache phones are shared by more than one name.
        let nk = norm_name(name);
        if nk.is_empty() {
            return picked;
        }
        let Some(ids) = self.by_name.get(&nk) else {
            return picked;
        };
        let city_lc = city.trim().to_lowercase();
        let phone10 = digits_last10(phone);
        let host = website_host(website);
        let street3 = street3_of_address(address);

        let city_ok = |r: &CacheRow| !city_lc.is_empty() && r.address_lc.contains(city_lc.as_str());
        let phone_ok = |r: &CacheRow| !phone10.is_empty() && phone10 == r.phone10;
        let host_ok = |r: &CacheRow| !host.is_empty() && host == r.host;
        let street_ok = |r: &CacheRow| !street3.is_empty() && street3 == r.street3;

        for arm in [
            &city_ok as &dyn Fn(&CacheRow) -> bool,
            &phone_ok,
            &host_ok,
            &street_ok,
        ] {
            if let Some(i) = self.latest(ids, arm) {
                picked.insert(i);
            }
        }
        picked
    }

    /// True when the cache holds any identity-matched record for this listing.
    fn has_record(
        &self,
        place_id: &str,
        name: &str,
        city: &str,
        phone: &str,
        website: &str,
        address: &str,
    ) -> bool {
        !self
            .matched(place_id, name, city, phone, website, address)
            .is_empty()
    }

    /// Union of the provider types from every arm that matches — never first-match-wins, so
    /// conflicting cached evidence still lands on `Verdict::Unknown` instead of silently picking a
    /// winner. Each arm contributes ONE record: its most recently cached match (place id as the
    /// tiebreak), so an older duplicate row for the same name cannot drag a stale trade into the
    /// union.
    fn types_for(
        &self,
        place_id: &str,
        name: &str,
        city: &str,
        phone: &str,
        website: &str,
        address: &str,
    ) -> Vec<String> {
        let picked = self.matched(place_id, name, city, phone, website, address);
        self.types_of(&picked)
    }

    /// The most recently cached record among `ids` that satisfies `pred`.
    fn latest(&self, ids: &[usize], pred: impl Fn(&CacheRow) -> bool) -> Option<usize> {
        ids.iter()
            .copied()
            .filter(|i| pred(&self.rows[*i]))
            .max_by(|a, b| {
                self.rows[*a]
                    .cached_ts
                    .total_cmp(&self.rows[*b].cached_ts)
                    .then_with(|| self.rows[*a].place_id.cmp(&self.rows[*b].place_id))
            })
    }

    fn types_of(&self, picked: &std::collections::BTreeSet<usize>) -> Vec<String> {
        let mut out: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for i in picked {
            out.extend(self.rows[*i].types.iter().cloned());
        }
        out.into_iter().collect()
    }
}

/// Lowercase, every non-alphanumeric run collapsed to one space, trimmed.
fn norm_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending_space = false;
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(c);
        } else {
            pending_space = true;
        }
    }
    out
}

/// Phone digits, last 10. Empty when the number has fewer than 10 digits.
fn digits_last10(phone: &str) -> String {
    let digits: String = phone.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 10 {
        String::new()
    } else {
        digits[digits.len() - 10..].to_string()
    }
}

/// Host only: scheme, leading `www.`, path, query and fragment stripped, lowercased.
fn website_host(website: &str) -> String {
    let mut s = website.trim().to_lowercase();
    for scheme in ["https://", "http://"] {
        if let Some(rest) = s.strip_prefix(scheme) {
            s = rest.to_string();
        }
    }
    let host = s
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('.')
        .to_string();
    host.strip_prefix("www.").unwrap_or(&host).to_string()
}

/// Street line (text before the first comma), normalised, first three tokens.
fn street3_of_address(address: &str) -> String {
    let line = address.split(',').next().unwrap_or("");
    norm_name(line)
        .split_whitespace()
        .take(3)
        .collect::<Vec<_>>()
        .join(" ")
}

fn split_csv(csv: &str) -> Vec<String> {
    csv.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// One active listing, with exactly the fields the guard judges it by.
#[derive(Debug, Clone)]
struct Listing {
    id: Uuid,
    directory_id: Option<Uuid>,
    name: String,
    place_id: String,
    phone: String,
    website: String,
    address: String,
    category: String,
    dir_city: String,
}

impl Listing {
    fn types(&self, places: &PlaceIndex) -> Vec<String> {
        places.types_for(
            &self.place_id,
            &self.name,
            &self.dir_city,
            &self.phone,
            &self.website,
            &self.address,
        )
    }

    fn has_record(&self, places: &PlaceIndex) -> bool {
        places.has_record(
            &self.place_id,
            &self.name,
            &self.dir_city,
            &self.phone,
            &self.website,
            &self.address,
        )
    }
}

async fn load_listings(db: &PgPool) -> Result<Vec<Listing>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT b.id, b.directory_id, b.name, coalesce(b.place_id, '') AS place_id, \
                coalesce(b.phone, '') AS phone, coalesce(b.website, '') AS website, \
                coalesce(b.address, '') AS address, \
                coalesce(dc.name, '') AS category, coalesce(d.city, '') AS dir_city \
         FROM businesses b \
         LEFT JOIN directory_categories dc ON dc.id = b.category_id \
         LEFT JOIN directories d ON d.id = b.directory_id \
         WHERE COALESCE(b.is_active, true) \
         ORDER BY b.name",
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .iter()
        .map(|row| Listing {
            id: row.try_get("id").unwrap_or_default(),
            directory_id: row.try_get("directory_id").unwrap_or(None),
            name: row.try_get("name").unwrap_or_default(),
            place_id: row.try_get("place_id").unwrap_or_default(),
            phone: row.try_get("phone").unwrap_or_default(),
            website: row.try_get("website").unwrap_or_default(),
            address: row.try_get("address").unwrap_or_default(),
            category: row.try_get("category").unwrap_or_default(),
            dir_city: row.try_get("dir_city").unwrap_or_default(),
        })
        .collect())
}

/// Walk every active listing. With `apply` false nothing is written — the caller prints the plan.
pub async fn run(db: &PgPool, apply: bool) -> Result<(Report, Vec<Change>), sqlx::Error> {
    let listings = load_listings(db).await?;
    let places = PlaceIndex::load(db).await?;

    let mut report = Report {
        scanned: listings.len(),
        ..Default::default()
    };
    let mut changes: Vec<Change> = Vec::new();
    let mut ids: Vec<(Uuid, Uuid, String)> = Vec::new(); // (business, directory, old category)

    for row in &listings {
        let id = row.id;
        let name = row.name.clone();
        let category = row.category.clone();
        let directory_id = row.directory_id;

        // A generic bucket ("Other", "Professional") is an honest label and is never checked; a
        // listing with no category at all has nothing to contradict.
        if category_guard::category_family(&category).is_none() {
            report.generic += 1;
            continue;
        }
        let types = row.types(&places);
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

/// The truth rule applied to the whole corpus: a listing whose trade no identity-grade evidence
/// names must not carry a trade at all.
///
/// `category_id` IS the decision ("this listing's trade is X"), so an undecided listing is one
/// with `category_id IS NULL`. Clearing it is what makes the rule hold on every surface at once —
/// page title, meta, H1, body copy, breadcrumb, category chips, the "More <trade>" heading and
/// membership of `/<city>/<trade>` all derive from that one column, so a cleared row behaves as a
/// local business everywhere without a second render path to keep in sync.
///
/// Evidence is the same union-of-arms provider record the re-file pass uses, plus the listing's own
/// name (`category_guard::is_decided`). Rows the evidence SUPPORTS are left untouched: "no
/// trade-bearing Google type" is not the same as "no evidence", and blind-clearing the class would
/// delete legitimate listings.
#[derive(Debug, Default)]
pub struct UndecidedReport {
    /// Active listings that carry a trade category — the class under judgement.
    pub scanned: usize,
    /// Legitimate: name or provider types name the trade. Left alone.
    pub decided: usize,
    /// No evidence at all. This count is the one that must not be written over blindly.
    pub undecided: usize,
    /// Evidence names a different trade; `audit-categories --apply` re-files these, not this pass.
    pub contradicted: usize,
    /// Generic buckets ("Other", "Professional") — an honest label, never a trade claim.
    pub generic: usize,
    /// Undecided AND no cached provider record exists for this identity (nothing could be known).
    pub no_record: usize,
    /// Categories cleared by this pass (only moves with `apply`).
    pub retired: usize,
    /// Stored paragraphs rewritten from "<trade> company" to the local-business form.
    pub copy_rewritten: usize,
}

/// One listing whose trade is being taken away, with the reason — evidence a human can re-check.
#[derive(Debug, Clone)]
pub struct Retired {
    pub name: String,
    pub city: String,
    pub category: String,
    pub reason: &'static str,
}

impl Retired {
    pub fn line(&self) -> String {
        format!(
            "{:<27} {:<46} {:<20} [{}]",
            self.category,
            self.name.chars().take(46).collect::<String>(),
            self.city.chars().take(20).collect::<String>(),
            self.reason
        )
    }
}

pub async fn retire_undecided(
    db: &PgPool,
    apply: bool,
) -> Result<(UndecidedReport, Vec<Retired>), sqlx::Error> {
    let listings = load_listings(db).await?;
    let places = PlaceIndex::load(db).await?;

    let mut report = UndecidedReport::default();
    let mut retired: Vec<Retired> = Vec::new();
    let mut ids: Vec<(Uuid, String)> = Vec::new(); // (business, old category)

    for row in &listings {
        if category_guard::category_family(&row.category).is_none() {
            if !row.category.trim().is_empty() {
                report.generic += 1;
            }
            continue;
        }
        report.scanned += 1;
        let types = row.types(&places);
        match category_guard::verdict(&row.name, &types, &row.category) {
            Verdict::Supported => report.decided += 1,
            Verdict::Contradicted(_) => report.contradicted += 1,
            Verdict::Unknown => {
                report.undecided += 1;
                let has_record = row.has_record(&places);
                if !has_record {
                    report.no_record += 1;
                }
                retired.push(Retired {
                    name: row.name.clone(),
                    city: row.dir_city.clone(),
                    category: row.category.clone(),
                    reason: if has_record {
                        "provider types name no trade"
                    } else {
                        "no provider record"
                    },
                });
                ids.push((row.id, row.category.clone()));
            }
        }
    }

    if !apply {
        return Ok((report, retired));
    }

    for (id, old_category) in ids {
        let cleared = sqlx::query(
            "UPDATE businesses SET category_id = NULL, updated_at = NOW() \
              WHERE id = $1 AND category_id IS NOT NULL",
        )
        .bind(id)
        .execute(db)
        .await
        .map(|r| r.rows_affected())
        .unwrap_or(0);
        if cleared == 0 {
            continue;
        }
        report.retired += 1;
        // The stored paragraph is the app's own render of the old category; it says
        // "<trade> company", so it has to be re-rendered now that the trade is gone. Only text
        // that is provably ours under the old category is touched.
        if crate::utils::business_copy::rewrite_for_category_change(db, id, Some(&old_category))
            .await
        {
            report.copy_rewritten += 1;
        }
    }

    Ok((report, retired))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cached(
        place_id: &str,
        name: &str,
        address: &str,
        phone: &str,
        website: &str,
        types: &[&str],
    ) -> CacheRow {
        CacheRow {
            place_id: place_id.to_string(),
            name_key: norm_name(name),
            address_lc: address.to_lowercase(),
            phone10: digits_last10(phone),
            host: website_host(website),
            street3: street3_of_address(address),
            cached_ts: 0.0,
            types: types.iter().map(|t| t.to_string()).collect(),
        }
    }

    /// The measured case (kanban t_0f61ea40): RealManage Winter Garden's cached record carries no
    /// trade-bearing Google type and its name carries no trade word. Nothing decides "Landscaping",
    /// so the listing must not claim it — and because the record EXISTS, the reason is "no trade",
    /// not "no record": the two are different and both are reportable.
    #[test]
    fn a_record_with_no_trade_type_decides_no_trade() {
        let index = PlaceIndex::build(vec![cached(
            "ChIJwS5zpMV-54gRbBb_9io52Pc",
            "RealManage Winter Garden",
            "270 W Plant St #340, Winter Garden, FL 34787, USA",
            "(866) 473-2573",
            "https://www.realmanage.com",
            &["service", "point_of_interest", "establishment"],
        )]);
        let types = index.types_for(
            "",
            "RealManage Winter Garden",
            "Winter Garden",
            "(866) 473-2573",
            "",
            "270 W Plant St #340",
        );
        assert!(
            index.has_record(
                "",
                "RealManage Winter Garden",
                "Winter Garden",
                "(866) 473-2573",
                "",
                ""
            ),
            "the cached record is the identity match"
        );
        assert!(!crate::utils::category_guard::is_decided(
            "RealManage Winter Garden",
            &types,
            "Landscaping"
        ));
    }

    /// A listing whose own name names the trade keeps it, generic provider types and all: absence
    /// of a Google trade type is not absence of evidence, and clearing these would be a wrong write.
    #[test]
    fn the_listings_own_name_decides_the_trade() {
        let types = vec![
            "service".to_string(),
            "point_of_interest".to_string(),
            "establishment".to_string(),
        ];
        assert!(crate::utils::category_guard::is_decided(
            "Palm Bay Lawn Landscaping",
            &types,
            "Landscaping"
        ));
        assert!(!crate::utils::category_guard::is_decided(
            "Palm Bay Lawn Landscaping",
            &types,
            "Pool Service"
        ));
    }

    fn sparky_row() -> CacheRow {
        cached(
            "pid-sparky",
            "Mister Sparky",
            "1200 Palm Bay Rd, Palm Bay, FL 32905",
            "(321) 555-0142",
            "https://www.mistersparky.com/palm-bay",
            &["electrician", "point_of_interest", "establishment"],
        )
    }

    #[test]
    fn name_normalisation_collapses_punctuation_runs() {
        assert_eq!(
            norm_name("  Chimento's  Spaghetti   House "),
            "chimento s spaghetti house"
        );
        assert_eq!(norm_name("PLUMB-PERFECT, LLC."), "plumb perfect llc");
        assert_eq!(
            norm_name("Conner & Associates LLC"),
            "conner associates llc"
        );
        assert_eq!(norm_name(""), "");
    }

    #[test]
    fn website_host_strips_scheme_www_and_path() {
        assert_eq!(
            website_host("https://www.MisterSparky.com/palm-bay?x=1"),
            "mistersparky.com"
        );
        assert_eq!(website_host("http://mistersparky.com"), "mistersparky.com");
        assert_eq!(website_host("mistersparky.com/"), "mistersparky.com");
        assert_eq!(website_host(""), "");
    }

    #[test]
    fn phone_key_is_the_last_ten_digits() {
        assert_eq!(digits_last10("+1 (321) 555-0142"), "3215550142");
        assert_eq!(digits_last10("321.555.0142"), "3215550142");
        assert_eq!(
            digits_last10("555-0142"),
            "",
            "short numbers are not an identity"
        );
        assert_eq!(digits_last10(""), "");
    }

    #[test]
    fn street_key_is_the_first_three_tokens_of_the_street_line() {
        assert_eq!(
            street3_of_address("1517 13th St, St Cloud, FL 34769"),
            "1517 13th st"
        );
        assert_eq!(street3_of_address("1517 13th St"), "1517 13th st");
        assert_eq!(street3_of_address(""), "");
    }

    #[test]
    fn each_arm_supplies_types_when_the_name_also_agrees() {
        let index = PlaceIndex::build(vec![sparky_row()]);

        // Arm 1: punctuation-insensitive name + the directory city in the cached address.
        assert_eq!(
            index.types_for("", "Mister Sparky", "Palm Bay", "", "", ""),
            vec!["electrician", "establishment", "point_of_interest"]
        );
        // Arm 2: name + phone (listing phone formatted differently from the cache).
        assert_eq!(
            index.types_for("", "MISTER SPARKY", "", "321-555-0142", "", ""),
            vec!["electrician", "establishment", "point_of_interest"]
        );
        // Arm 3: name + website host (listing URL is the bare www form).
        assert_eq!(
            index.types_for("", "Mister Sparky", "", "", "www.mistersparky.com", ""),
            vec!["electrician", "establishment", "point_of_interest"]
        );
        // Arm 4: name + first three street tokens.
        assert_eq!(
            index.types_for(
                "",
                "Mister Sparky",
                "",
                "",
                "",
                "9999 Palm Bay Rd, Palm Bay"
            ),
            Vec::<String>::new(),
            "a differing street line is not equality"
        );
        assert_eq!(
            index.types_for(
                "",
                "Mister Sparky",
                "",
                "",
                "",
                "1200 Palm Bay Rd, Palm Bay"
            ),
            vec!["electrician", "establishment", "point_of_interest"]
        );
    }

    /// The card's required proof: shared phone numbers are not identity. Same phone, different
    /// name must supply NO evidence, and the guard must therefore say Unknown, not Contradicted.
    #[test]
    fn a_phone_match_alone_supplies_no_evidence() {
        let index = PlaceIndex::build(vec![sparky_row()]);
        let types = index.types_for("", "Tiffany Licata PA", "", "321-555-0142", "", "");
        assert!(types.is_empty(), "phone alone leaked types: {types:?}");
        assert!(matches!(
            category_guard::verdict("Tiffany Licata PA", &types, "Personal Injury Lawyer"),
            Verdict::Unknown
        ));
    }

    #[test]
    fn a_website_or_street_match_alone_supplies_no_evidence() {
        let index = PlaceIndex::build(vec![sparky_row()]);
        assert!(index
            .types_for("", "Mister Plumber", "", "", "mistersparky.com", "")
            .is_empty());
        assert!(index
            .types_for("", "Mister Plumber", "", "", "", "1200 Palm Bay Rd")
            .is_empty());
    }

    #[test]
    fn a_city_in_the_address_without_name_equality_supplies_no_evidence() {
        let index = PlaceIndex::build(vec![sparky_row()]);
        assert!(index
            .types_for("", "Some Other Electrician", "Palm Bay", "", "", "")
            .is_empty());
    }

    #[test]
    fn place_id_arm_does_not_require_the_name() {
        let index = PlaceIndex::build(vec![sparky_row()]);
        assert_eq!(
            index.types_for("pid-sparky", "Renamed Since Import", "", "", "", ""),
            vec!["electrician", "establishment", "point_of_interest"]
        );
    }

    /// Union, not first-match-wins: two cache rows for the same name, each reached by a different
    /// identifier, contribute both type sets so conflicting evidence still reads as conflicting.
    #[test]
    fn types_from_every_matching_arm_are_unioned() {
        let index = PlaceIndex::build(vec![
            cached(
                "pid-a",
                "TotalScape Design",
                "9 Palm Bay Rd, Palm Bay, FL",
                "321-555-0100",
                "totalscape.example",
                &["swimming_pool", "general_contractor"],
            ),
            cached(
                "pid-b",
                "Totalscape Design",
                "77 Orlando Ave, Orlando, FL",
                "",
                "https://totalscape.example/about",
                &["beauty_salon", "spa"],
            ),
        ]);
        // Reached by phone on row A and by host on row B; neither shares the other's identifier.
        let types = index.types_for(
            "",
            "TotalScape Design",
            "",
            "(321) 555-0100",
            "totalscape.example",
            "",
        );
        for expected in ["swimming_pool", "general_contractor", "beauty_salon", "spa"] {
            assert!(
                types.iter().any(|t| t == expected),
                "missing {expected} in {types:?}"
            );
        }
    }
}
