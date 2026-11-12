//! Deterministic, per-listing business descriptions (kanban t_6b831daa).
//!
//! Measured 2026-11-12: 4,099 of 4,100 active listings had a NULL `description`, so every
//! business detail page rendered one canned fallback sentence and nothing else — 4,099
//! near-duplicate pages. Paying an LLM to author 4,100 paragraphs is a spend decision that
//! belongs to the owner, so this module does what is free and factual instead: it composes a
//! short paragraph out of facts the row ALREADY holds — trade, locality, rating, review count,
//! street address and contact route — and rotates the wording deterministically on the listing
//! id so no two pages share a paragraph.
//!
//! Hard rules encoded here:
//!   * never invent a fact (no hours, prices, staff, history or claims we cannot prove);
//!   * never overwrite copy a business owner or admin wrote (`ensure_description` only fills a
//!     blank column);
//!   * return `None` rather than a hollow sentence when there is nothing real to say.

use sqlx::{PgPool, Row};
use uuid::Uuid;

/// The facts a description may state. Every field except `name` is optional because real
/// imports are incomplete.
pub struct Facts<'a> {
    pub name: &'a str,
    pub category: Option<&'a str>,
    pub city: &'a str,
    pub state: &'a str,
    pub address: Option<&'a str>,
    pub zip: Option<&'a str>,
    pub phone: Option<&'a str>,
    pub website: Option<&'a str>,
    pub rating: Option<f64>,
    pub review_count: i32,
}

fn non_empty(v: Option<&str>) -> Option<&str> {
    v.map(str::trim).filter(|s| !s.is_empty())
}

fn comma(n: i32) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// "a" / "an" for the trade phrase. Acronyms spelled letter-by-letter (HVAC, HVACR) take "an"
/// when the first letter is pronounced with a vowel sound.
fn article(trade: &str) -> &'static str {
    let mut chars = trade.chars();
    let Some(first) = chars.next() else {
        return "a";
    };
    let second_upper = chars
        .next()
        .map(|c| c.is_ascii_uppercase())
        .unwrap_or(false);
    let lower = first.to_ascii_lowercase();
    if second_upper {
        // Acronym: judge it by how the first LETTER is pronounced.
        return if matches!(
            lower,
            'a' | 'e' | 'f' | 'h' | 'i' | 'l' | 'm' | 'n' | 'o' | 'r' | 's' | 'x'
        ) {
            "an"
        } else {
            "a"
        };
    }
    if matches!(lower, 'a' | 'e' | 'i' | 'o' | 'u') {
        "an"
    } else {
        "a"
    }
}

/// Trade phrase for the opener: lowercase the category unless it is an acronym.
fn trade_phrase(category: Option<&str>) -> Option<String> {
    let c = non_empty(category)?;
    let letters: Vec<char> = c.chars().filter(|ch| ch.is_alphabetic()).collect();
    let is_acronym = !letters.is_empty() && letters.iter().all(|ch| ch.is_ascii_uppercase());
    if is_acronym {
        return Some(c.to_string());
    }
    let lower = c.to_lowercase();
    // "Personal Injury Lawyer" reads better as "... works as a personal injury lawyer"; a
    // plural trade ("Bakeries & Dessert Shops") keeps its plural and drops the article.
    Some(lower)
}

fn is_plural(trade: &str) -> bool {
    trade
        .split_whitespace()
        .last()
        .map(|w| w.ends_with('s') || w.ends_with('&'))
        .unwrap_or(false)
}

/// Trim Google's trailing ", USA" so the address reads like an address.
fn tidy_address(address: &str) -> String {
    let trimmed = address.trim().trim_end_matches(',');
    let trimmed = trimmed.strip_suffix(", USA").unwrap_or(trimmed).trim();
    trimmed.trim_end_matches(',').trim().to_string()
}

/// Compose one description. `seed` only rotates phrasing (pass a stable per-listing value such
/// as the id's hash so the same row always reads the same way).
///
/// Returns `None` when the row has no name or no locality — better a missing description than a
/// sentence with nothing in it.
pub fn compose(f: &Facts, seed: u64) -> Option<String> {
    let name = non_empty(Some(f.name))?;
    let city = non_empty(Some(f.city));
    let state = non_empty(Some(f.state));
    let place = match (city, state) {
        (Some(c), Some(s)) => Some(format!("{c}, {s}")),
        (Some(c), None) => Some(c.to_string()),
        (None, Some(s)) => Some(s.to_string()),
        (None, None) => None,
    };
    let trade = trade_phrase(f.category);
    let mut out: Vec<String> = Vec::new();
    let v = (seed % 3) as usize;

    match (&trade, &place) {
        (Some(t), Some(p)) if !is_plural(t) => {
            let art = article(t);
            out.push(match v {
                0 => format!("{name} is {art} {t} in {p}."),
                1 => format!("{name} is {art} {t} serving {p}."),
                _ => format!("{name} is listed as {art} {t} in {p}."),
            });
        }
        (Some(t), Some(p)) => out.push(match v {
            0 => format!("{name} is listed in {p} under {t}."),
            1 => format!("{name} is a {t} listing serving {p}."),
            _ => format!("{name} in {p} is filed under {t}."),
        }),
        (None, Some(p)) => out.push(match v {
            0 => format!("{name} is a local business in {p}."),
            1 => format!("{name} is based in {p}."),
            _ => format!("{name} operates in {p}."),
        }),
        (Some(t), None) if !is_plural(t) => {
            out.push(format!("{name} is {} {t}.", article(t)));
        }
        (Some(t), None) => out.push(format!("{name} is listed under {t}.")),
        (None, None) => out.push(format!("{name} is listed in the ZaarHub directory.")),
    }

    // Reputation — only the number the row actually carries.
    match (f.rating.filter(|r| *r > 0.0), f.review_count) {
        (Some(r), n) if n > 0 => out.push(match v {
            1 => format!(
                "Customers rate it {r:.1} out of 5 across {} Google reviews.",
                comma(n)
            ),
            2 => format!(
                "It holds {} Google reviews with an average of {r:.1} stars.",
                comma(n)
            ),
            _ => format!(
                "It carries a {r:.1}-star rating from {} Google reviews.",
                comma(n)
            ),
        }),
        (Some(r), _) => out.push(format!("It is rated {r:.1} stars.")),
        (None, n) if n > 0 => out.push(format!("It has {} Google reviews on file.", comma(n))),
        _ => {}
    }

    // Street address.
    if let Some(a) = non_empty(f.address) {
        let mut addr = tidy_address(a);
        if let Some(z) = non_empty(f.zip) {
            if !addr.contains(z) {
                addr = format!("{addr}, {z}");
            }
        }
        if !addr.is_empty() {
            out.push(match v {
                1 => format!("Its address is {addr}."),
                2 => format!("The listing is at {addr}."),
                _ => format!("You will find it at {addr}."),
            });
        }
    }

    // Contact route.
    let phone = non_empty(f.phone);
    let site = non_empty(f.website).map(|w| crate::utils::url_cleaner::clean_url_pair(w).1);
    match (phone, site) {
        (Some(p), Some(s)) => out.push(format!("Reach them on {p}, or see more at {s}.")),
        (Some(p), None) => out.push(format!("Call {p} for details.")),
        (None, Some(s)) => out.push(format!("More details at {s}.")),
        (None, None) => {}
    }

    let text = out.join(" ");
    if text.split_whitespace().count() < 12 {
        // Not enough real facts to stand as page copy.
        return None;
    }
    Some(text)
}

/// Stable phrasing seed for a listing.
pub fn seed_for(id: Uuid) -> u64 {
    let bytes = id.as_bytes();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

/// Fill a listing's description from its own facts, but ONLY when the column is blank.
/// Returns true when a row was written.
pub async fn ensure_description(db: &PgPool, id: Uuid) -> bool {
    let row = sqlx::query(
        "SELECT b.name, b.description, \
                COALESCE(NULLIF(btrim(b.city), ''), d.city, '') AS city, \
                COALESCE(NULLIF(btrim(b.state), ''), d.state, '') AS state, \
                b.address, b.zip, b.phone, b.website, b.rating, b.review_count, \
                c.name AS category \
         FROM businesses b \
         LEFT JOIN directory_categories c ON c.id = b.category_id \
         LEFT JOIN directories d ON d.id = b.directory_id \
         WHERE b.id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await;

    let row = match row {
        Ok(Some(r)) => r,
        Ok(None) => return false,
        Err(e) => {
            tracing::warn!("[description] could not read listing {id}: {e}");
            return false;
        }
    };

    let existing: Option<String> = row.try_get("description").unwrap_or(None);
    if existing.map(|d| !d.trim().is_empty()).unwrap_or(false) {
        return false;
    }

    let name: String = row.try_get("name").unwrap_or_default();
    let city: String = row.try_get("city").unwrap_or_default();
    let state: String = row.try_get("state").unwrap_or_default();
    let address: Option<String> = row.try_get("address").unwrap_or(None);
    let zip: Option<String> = row.try_get("zip").unwrap_or(None);
    let phone: Option<String> = row.try_get("phone").unwrap_or(None);
    let website: Option<String> = row.try_get("website").unwrap_or(None);
    let rating: Option<f64> = row.try_get("rating").unwrap_or(None);
    let review_count: i32 = row.try_get("review_count").unwrap_or(0);
    let category: Option<String> = row.try_get("category").unwrap_or(None);

    let facts = Facts {
        name: &name,
        category: category.as_deref(),
        city: &city,
        state: &state,
        address: address.as_deref(),
        zip: zip.as_deref(),
        phone: phone.as_deref(),
        website: website.as_deref(),
        rating,
        review_count,
    };
    let Some(text) = compose(&facts, seed_for(id)) else {
        return false;
    };

    match sqlx::query(
        "UPDATE businesses SET description = $2 \
         WHERE id = $1 AND (description IS NULL OR btrim(description) = '')",
    )
    .bind(id)
    .bind(&text)
    .execute(db)
    .await
    {
        Ok(r) => r.rows_affected() > 0,
        Err(e) => {
            tracing::warn!("[description] could not write listing {id}: {e}");
            false
        }
    }
}

/// Fill every blank, live description in the current batch. `limit` caps one pass so the sweep
/// stays cheap; callers loop until it returns 0.
pub async fn backfill_blank(db: &PgPool, limit: i64) -> usize {
    let ids: Vec<Uuid> = match sqlx::query_scalar(
        "SELECT b.id FROM businesses b \
         WHERE COALESCE(b.is_active, true) AND COALESCE(b.status, 'active') = 'active' \
           AND (b.description IS NULL OR btrim(b.description) = '') \
         ORDER BY b.id LIMIT $1",
    )
    .bind(limit)
    .fetch_all(db)
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("[description] backfill could not list blank rows: {e}");
            return 0;
        }
    };

    let mut written = 0usize;
    for id in ids {
        if ensure_description(db, id).await {
            written += 1;
        }
    }
    if written > 0 {
        tracing::info!("[description] filled {written} blank listing descriptions");
    }
    written
}

/// Keep descriptions complete without a human step: one pass at boot, then every 30 minutes for
/// listings imported or created since (a blank column is a defect, not a state).
pub fn start_description_backfill(db: PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1800));
        loop {
            interval.tick().await;
            // Bounded passes so one sweep can never hold the pool for long.
            for _ in 0..8 {
                if backfill_blank(&db, 500).await == 0 {
                    break;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(name: &'a str, cat: Option<&'a str>) -> Facts<'a> {
        Facts {
            name,
            category: cat,
            city: "Apopka",
            state: "FL",
            address: Some("931 N State Rd 434 #1225, Altamonte Springs, FL 32714, USA"),
            zip: Some("32714"),
            phone: Some("(689) 258-2122"),
            website: Some("https://www.fineinkstudios.com/?utm_source=googleplaces"),
            rating: Some(5.0),
            review_count: 1124,
        }
    }

    #[test]
    fn states_only_facts_from_the_row() {
        let text = compose(&facts("Fine Ink Studios", Some("Tattoo Parlor")), 7).unwrap();
        assert!(text.contains("Fine Ink Studios"));
        assert!(text.contains("tattoo parlor"));
        assert!(text.contains("Apopka, FL"));
        assert!(text.contains("5.0"));
        assert!(text.contains("1,124"));
        assert!(text.contains("(689) 258-2122"));
        assert!(text.contains("fineinkstudios.com"));
        assert!(!text.contains("utm_source"));
        assert!(!text.contains("USA"));
    }

    #[test]
    fn wording_rotates_so_pages_are_not_identical() {
        let a = compose(&facts("Fine Ink Studios", Some("Tattoo Parlor")), 0).unwrap();
        let b = compose(&facts("Fine Ink Studios", Some("Tattoo Parlor")), 1).unwrap();
        let c = compose(&facts("Fine Ink Studios", Some("Tattoo Parlor")), 2).unwrap();
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
    }

    #[test]
    fn acronym_trades_keep_their_case_and_article() {
        let text = compose(&facts("Whale Roofing", Some("HVAC")), 0).unwrap();
        assert!(text.contains("an HVAC"), "{text}");
        assert!(!text.contains("hvac"));
    }

    #[test]
    fn no_name_means_no_description() {
        assert!(compose(&facts("   ", Some("Dentist")), 0).is_none());
    }

    #[test]
    fn sparse_row_still_reads_as_facts() {
        let f = Facts {
            name: "Whale Roofing & Construction",
            category: Some("Roofing"),
            city: "Boca Raton",
            state: "FL",
            address: None,
            zip: None,
            phone: Some("(561) 467-7946"),
            website: None,
            rating: Some(5.0),
            review_count: 341,
        };
        let text = compose(&f, 3).unwrap();
        assert!(text.contains("Whale Roofing & Construction"));
        assert!(text.contains("341"));
        assert!(text.contains("(561) 467-7946"));
    }
}
