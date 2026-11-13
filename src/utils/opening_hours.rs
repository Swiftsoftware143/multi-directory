//! Opening hours — parse Google's cached weekly periods, and render them once (kanban t_2b630e2a).
//!
//! Measured 2026-11-13 on the live DB: `google_places_cache.opening_hours` carries real weekly
//! periods for 4,390 of 4,499 cached places, and 3,898 of 4,100 live listings resolve to one of
//! them (place_id first, then normalised name + street line). Nothing rendered them: `businesses`
//! has no hours column and the detail page never read the cache, so ~4,100 pages told a visitor
//! nothing about when the business is open while the data had already been paid for.
//!
//! One parser, one renderer. The page block, the JSON-LD `openingHoursSpecification` and the
//! description fact set all read this type, so they cannot disagree with each other.
//!
//! Rules encoded here:
//!   * never invent a time — a day Google publishes no period for is "Closed";
//!   * a close-less period is only read as 24/7 when it is the single period opening at 00:00
//!     (that is exactly how Google encodes "Open 24 hours"); anything else is dropped, not guessed;
//!   * a close on a later day keeps its real next-day time (a bar closing 2 AM reads
//!     "11 AM – 2 AM"), instead of being truncated to midnight.

use serde_json::Value;
use sqlx::PgPool;

/// 0 = Monday .. 6 = Sunday. Google's `periods[].open.day` is 0 = Sunday.
pub const DAY_LABELS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

const MINUTES_PER_DAY: i32 = 1440;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span {
    /// Minutes from midnight on the day this span belongs to.
    pub open: i32,
    /// Minutes from midnight; may exceed 1440 when the business closes after midnight.
    pub close: i32,
}

impl Span {
    fn text(&self) -> String {
        format!("{} – {}", fmt_time(self.open), fmt_time(self.close))
    }
}

#[derive(Clone, Debug)]
pub struct Week {
    /// Monday first.
    pub days: [Vec<Span>; 7],
}

fn minutes(v: &Value) -> Option<i32> {
    let h = v.get("hour")?.as_i64()?;
    let m = v.get("minute").and_then(Value::as_i64).unwrap_or(0);
    Some(h as i32 * 60 + m as i32)
}

/// Parse the cached Google payload (`{ "periods": [ … ], … }`). `None` when there is nothing
/// truthful to render.
pub fn from_google(v: &Value) -> Option<Week> {
    let periods = v.get("periods").and_then(Value::as_array)?;
    if periods.is_empty() {
        return None;
    }

    // Google publishes "Open 24 hours" as a single period that opens at 00:00 with no close.
    if periods.len() == 1 {
        let p = &periods[0];
        let open_zero = p
            .get("open")
            .and_then(|o| o.get("hour"))
            .and_then(Value::as_i64)
            == Some(0)
            && p.get("open")
                .and_then(|o| o.get("minute"))
                .and_then(Value::as_i64)
                .unwrap_or(0)
                == 0;
        let no_close = p.get("close").map(Value::is_null).unwrap_or(true);
        if open_zero && no_close {
            return Some(Week {
                days: std::array::from_fn(|_| {
                    vec![Span {
                        open: 0,
                        close: MINUTES_PER_DAY,
                    }]
                }),
            });
        }
    }

    let mut days: [Vec<Span>; 7] = std::array::from_fn(|_| Vec::new());
    for p in periods {
        let (Some(o), Some(c)) = (p.get("open"), p.get("close")) else {
            continue;
        };
        if c.is_null() {
            // A close-less period that is not the 24/7 form: refuse to guess.
            continue;
        }
        let (Some(od), Some(om), Some(cd), Some(cm)) = (
            o.get("day").and_then(Value::as_i64),
            minutes(o),
            c.get("day").and_then(Value::as_i64),
            minutes(c),
        ) else {
            continue;
        };
        if !(0..7).contains(&od) || !(0..7).contains(&cd) || om >= MINUTES_PER_DAY {
            continue;
        }
        let close = if cd == od {
            if cm <= om {
                MINUTES_PER_DAY
            } else {
                cm
            }
        } else {
            let ahead = (cd - od).rem_euclid(7) as i32;
            ahead * MINUTES_PER_DAY + cm
        };
        let idx = ((od + 6) % 7) as usize;
        days[idx].push(Span { open: om, close });
    }
    for d in days.iter_mut() {
        d.sort_by_key(|s| s.open);
        d.dedup();
    }
    if days.iter().all(|d| d.is_empty()) {
        return None;
    }
    Some(Week { days })
}

/// "11 AM", "4:30 PM", "12 AM" (midnight). Times past midnight wrap for display.
pub fn fmt_time(mins: i32) -> String {
    let m = mins.rem_euclid(MINUTES_PER_DAY);
    let (h24, mm) = (m / 60, m % 60);
    let (h12, suffix) = match h24 {
        0 => (12, "AM"),
        h if h < 12 => (h, "AM"),
        12 => (12, "PM"),
        h => (h - 12, "PM"),
    };
    if mm == 0 {
        format!("{h12} {suffix}")
    } else {
        format!("{h12}:{mm:02} {suffix}")
    }
}

impl Week {
    pub fn is_24_7(&self) -> bool {
        self.days
            .iter()
            .all(|d| d.len() == 1 && d[0].open == 0 && d[0].close == MINUTES_PER_DAY)
    }

    /// Monday..Sunday, "Closed" for a day with no period. Never empty.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        DAY_LABELS
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let d = &self.days[i];
                let text = if d.is_empty() {
                    "Closed".to_string()
                } else {
                    d.iter().map(Span::text).collect::<Vec<_>>().join(", ")
                };
                (*label, text)
            })
            .collect()
    }

    /// The page block. Empty string when there is nothing to render — callers insert it as-is,
    /// so a listing without hours shows no block and no placeholder.
    pub fn html(&self) -> String {
        if self.rows().iter().all(|(_, t)| t == "Closed") {
            return String::new();
        }
        if self.is_24_7() {
            return "<div class=\"row hours\"><strong>Hours</strong><div style=\"margin-top:6px\">🕒 Open 24 hours</div></div>"
                .to_string();
        }
        let mut cells = String::new();
        for (day, text) in self.rows() {
            let style = if text == "Closed" {
                " style=\"color:#6b7280\""
            } else {
                ""
            };
            cells.push_str(&format!(
                "<div class=\"hours-day\">{}</div><div class=\"hours-time\"{}>{}</div>",
                day, style, text
            ));
        }
        format!(
            "<div class=\"row hours\"><strong>Hours</strong>\
<div style=\"display:grid;grid-template-columns:max-content 1fr;gap:3px 20px;margin-top:8px;font-size:.95rem\">\
{}</div></div>",
            cells
        )
    }

    /// `openingHoursSpecification` for the existing LocalBusiness JSON-LD, grouped so identical
    /// days share one entry. Days that are closed contribute nothing.
    pub fn jsonld(&self) -> Option<Value> {
        let mut groups: Vec<(Vec<&'static str>, Span)> = Vec::new();
        for (i, d) in self.days.iter().enumerate() {
            for s in d {
                match groups.iter_mut().find(|(_, g)| g == s) {
                    Some((days, _)) => days.push(DAY_LABELS[i]),
                    None => groups.push((vec![DAY_LABELS[i]], *s)),
                }
            }
        }
        if groups.is_empty() {
            return None;
        }
        let specs: Vec<Value> = groups
            .into_iter()
            .map(|(days, s)| {
                let closes = if s.close >= MINUTES_PER_DAY && s.close % MINUTES_PER_DAY == 0 {
                    "23:59".to_string()
                } else {
                    hhmm(s.close)
                };
                let day_value: Value = if days.len() == 1 {
                    Value::String(days[0].to_string())
                } else {
                    Value::Array(days.iter().map(|d| Value::String(d.to_string())).collect())
                };
                serde_json::json!({
                    "@type": "OpeningHoursSpecification",
                    "dayOfWeek": day_value,
                    "opens": hhmm(s.open),
                    "closes": closes,
                })
            })
            .collect();
        Some(Value::Array(specs))
    }

    /// One short factual phrase for the description generator, always carrying the times:
    /// "open 11 AM – 11 PM seven days a week", "open 8 AM – 8 PM on weekdays and closed on
    /// Saturday and Sunday", "open around the clock". `None` when nothing is open at all.
    pub fn summary(&self) -> Option<String> {
        if self.is_24_7() {
            return Some("open around the clock".to_string());
        }
        // Days that share a schedule are described together.
        let mut groups: Vec<(Vec<Span>, Vec<&'static str>)> = Vec::new();
        for (i, d) in self.days.iter().enumerate() {
            if d.is_empty() {
                continue;
            }
            match groups.iter_mut().find(|(g, _)| g == d) {
                Some((_, days)) => days.push(DAY_LABELS[i]),
                None => groups.push((d.clone(), vec![DAY_LABELS[i]])),
            }
        }
        if groups.is_empty() {
            return None;
        }
        groups.sort_by_key(|(_, days)| std::cmp::Reverse(days.len()));
        let (spans, days) = &groups[0];
        let hours = spans.iter().map(Span::text).collect::<Vec<_>>().join(", ");
        let mut phrase = format!("open {hours} {}", day_phrase(days));
        let closed: Vec<&str> = DAY_LABELS
            .iter()
            .enumerate()
            .filter(|(i, _)| self.days[*i].is_empty())
            .map(|(_, l)| *l)
            .collect();
        match closed.len() {
            0 => {
                if groups.len() > 1 {
                    let others: Vec<&str> = groups[1..]
                        .iter()
                        .flat_map(|(_, d)| d.iter().copied())
                        .collect();
                    phrase.push_str(&format!(", with different hours on {}", list_and(&others)));
                }
            }
            _ => phrase.push_str(&format!(" and closed on {}", list_and(&closed))),
        }
        Some(phrase)
    }
}

/// "a", "a and b", "a, b and c".
fn list_and(items: &[&'static str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {}", rest.join(", "), last),
    }
}

/// "seven days a week", "on weekdays", "Monday to Friday", "on Monday".
fn day_phrase(days: &[&'static str]) -> String {
    let mut idx: Vec<usize> = days
        .iter()
        .filter_map(|d| DAY_LABELS.iter().position(|x| x == d))
        .collect();
    idx.sort_unstable();
    if idx.len() == 7 {
        return "seven days a week".to_string();
    }
    if idx == [0, 1, 2, 3, 4] {
        return "on weekdays".to_string();
    }
    let consecutive = idx.windows(2).all(|w| w[1] == w[0] + 1);
    if idx.len() >= 3 && consecutive {
        return format!(
            "{} to {}",
            DAY_LABELS[idx[0]],
            DAY_LABELS[idx[idx.len() - 1]]
        );
    }
    if idx.len() == 1 {
        return format!("on {}", DAY_LABELS[idx[0]]);
    }
    let names: Vec<&str> = idx.iter().map(|i| DAY_LABELS[*i]).collect();
    let (last, rest) = names.split_last().expect("non-empty");
    format!("on {} and {}", rest.join(", "), last)
}

fn hhmm(mins: i32) -> String {
    let m = mins.rem_euclid(MINUTES_PER_DAY);
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// Resolve a listing to its cached hours. Read-only; no row is written and no Google call is made.
///
/// Linkage, measured 2026-11-13: `businesses.place_id` exists on only 353 of 4,100 live rows, so
/// place_id alone reaches ~6% of pages. The cache and the directory were filled from the same
/// source, so normalised name + the listing's street line reaches 3,885 and name + city 3,635;
/// place_id is tried first because it is exact, then name + street, then name + city as a fallback
/// for rows with no street. The city guard keeps an identically named business in another city
/// from lending its hours (the cache holds real duplicate names).
pub async fn lookup(
    db: &PgPool,
    place_id: Option<&str>,
    name: &str,
    street: Option<&str>,
    city: &str,
) -> Option<Week> {
    let name_key = name.trim().to_lowercase();
    if name_key.is_empty() {
        return None;
    }
    let street_key = street
        .map(|s| s.split(',').next().unwrap_or("").trim().to_lowercase())
        .unwrap_or_default();
    let city_key = city.trim().to_lowercase();

    let raw: Option<Value> = sqlx::query_scalar(
        r#"SELECT c.opening_hours
           FROM google_places_cache c
           WHERE c.opening_hours IS NOT NULL
             AND jsonb_typeof(c.opening_hours) = 'object'
             AND c.opening_hours ? 'periods'
             AND (
               ($1::text IS NOT NULL AND c.place_id = $1::text)
               OR (
                 lower(c.name) = $2
                 AND (
                   ($3::text <> '' AND position($3 in lower(coalesce(c.formatted_address, ''))) > 0)
                   OR ($3::text = '' AND $4::text <> '' AND position($4 in lower(coalesce(c.formatted_address, ''))) > 0)
                 )
               )
             )
           ORDER BY (c.place_id = $1::text) DESC NULLS LAST,
                    (position($3 in lower(coalesce(c.formatted_address, ''))) > 0) DESC,
                    c.cached_at DESC
           LIMIT 1"#,
    )
    .bind(place_id)
    .bind(&name_key)
    .bind(&street_key)
    .bind(&city_key)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();

    from_google(&raw?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({"periods": [
            {"open": {"day": 1, "hour": 9, "minute": 0}, "close": {"day": 1, "hour": 17, "minute": 0}},
            {"open": {"day": 2, "hour": 9, "minute": 0}, "close": {"day": 2, "hour": 17, "minute": 0}},
            {"open": {"day": 3, "hour": 9, "minute": 30}, "close": {"day": 3, "hour": 18, "minute": 0}},
            {"open": {"day": 5, "hour": 11, "minute": 0}, "close": {"day": 6, "hour": 2, "minute": 0}}
        ]})
    }

    #[test]
    fn parses_week_and_marks_closed_days() {
        let w = from_google(&sample()).expect("hours");
        let rows = w.rows();
        assert_eq!(rows[0].0, "Monday");
        assert_eq!(rows[0].1, "9 AM – 5 PM");
        assert_eq!(rows[2].1, "9:30 AM – 6 PM");
        assert_eq!(rows[3].1, "Closed"); // Thursday is absent from the payload
        assert_eq!(rows[4].1, "11 AM – 2 AM"); // Friday closes 2 AM Saturday
        assert_eq!(rows[5].1, "Closed"); // Saturday has no opening period
        assert_eq!(rows[6].1, "Closed"); // Sunday is absent too
        assert!(w.html().contains("Hours"));
        assert!(w.html().contains("Closed"));
    }

    #[test]
    fn empty_payload_renders_nothing() {
        assert!(from_google(&json!({"periods": []})).is_none());
        assert!(from_google(&json!({})).is_none());
        assert!(from_google(&Value::Null).is_none());
    }

    #[test]
    fn twenty_four_hours_is_one_period_without_close() {
        let w = from_google(&json!({"periods": [
            {"open": {"day": 0, "hour": 0, "minute": 0}}
        ]}))
        .expect("24/7");
        assert!(w.is_24_7());
        let html = w.html();
        assert!(html.contains("Open 24 hours"));
        assert!(!html.contains("Closed"));
        let ld = w.jsonld().expect("ld");
        assert_eq!(ld[0]["opens"], "00:00");
        assert_eq!(ld[0]["closes"], "23:59");
    }

    #[test]
    fn jsonld_groups_identical_days() {
        let w = from_google(&json!({"periods": [
            {"open": {"day": 1, "hour": 9, "minute": 0}, "close": {"day": 1, "hour": 17, "minute": 0}},
            {"open": {"day": 2, "hour": 9, "minute": 0}, "close": {"day": 2, "hour": 17, "minute": 0}}
        ]}))
        .expect("hours");
        let ld = w.jsonld().expect("ld");
        let days = ld[0]["dayOfWeek"].as_array().expect("array");
        assert_eq!(days.len(), 2);
        assert_eq!(ld[0]["closes"], "17:00");
    }

    #[test]
    fn midnight_close_reads_as_midnight() {
        let w = from_google(&json!({"periods": [
            {"open": {"day": 5, "hour": 11, "minute": 0}, "close": {"day": 6, "hour": 0, "minute": 0}}
        ]}))
        .expect("hours");
        assert_eq!(w.rows()[4].1, "11 AM – 12 AM");
        let ld = w.jsonld().expect("ld");
        assert_eq!(ld[0]["closes"], "23:59");
    }

    #[test]
    fn summary_is_factual() {
        let w = from_google(&sample()).expect("hours");
        let s = w.summary().expect("summary");
        assert!(!s.is_empty());
        let daily = from_google(&json!({"periods": (1..=7).map(|d| json!({
            "open": {"day": d % 7, "hour": 8, "minute": 0},
            "close": {"day": d % 7, "hour": 20, "minute": 0}
        })).collect::<Vec<_>>()}))
        .expect("daily");
        assert_eq!(
            daily.summary().unwrap(),
            "open 8 AM – 8 PM seven days a week"
        );
    }
}
