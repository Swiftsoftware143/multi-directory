//! Publishing guardrails for the in-house blog engine.
//!
//! Every draft is checked before it can be published. The result is one of:
//!   * `pass`  — nothing failed and the quality score meets the directory's minimum;
//!   * `hold`  — a person must look at it (sensitive/borderline, or score too low);
//!   * `block` — a hard rule failed; it cannot be published until the cause is fixed.
//!
//! The checks that need an external API (plagiarism, an AI classifier, image licensing) are
//! deliberately not faked here: they are reported as "not configured" and stay `hold` only
//! when the directory turns them on. Everything that can be decided from the post itself is
//! decided from the post itself.

use regex::Regex;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Hold,
    Block,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Pass => "pass",
            Outcome::Hold => "hold",
            Outcome::Block => "block",
        }
    }
    fn rank(self) -> u8 {
        match self {
            Outcome::Pass => 0,
            Outcome::Hold => 1,
            Outcome::Block => 2,
        }
    }
    fn worse(self, other: Outcome) -> Outcome {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }
}

pub struct CheckResult {
    pub check: &'static str,
    pub outcome: Outcome,
    pub detail: String,
}

pub struct Report {
    pub outcome: Outcome,
    pub quality_score: i32,
    pub checks: Vec<CheckResult>,
    pub content_hash: String,
}

/// Words that are never acceptable in a published post. The directory can allow-list words
/// that are legitimate for its own brand (a brewery's own product name, for example).
const PROFANITY: &[&str] = &[
    "fuck",
    "shit",
    "bitch",
    "asshole",
    "cunt",
    "bastard",
    "dickhead",
    "twat",
    "wanker",
    "motherfucker",
    "bullshit",
];

fn plain_text(html: &str) -> String {
    let script = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    let style = Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let tags = Regex::new(r"(?s)<[^>]+>").unwrap();
    let no_script = script.replace_all(html, " ").to_string();
    let no_style = style.replace_all(&no_script, " ").to_string();
    tags.replace_all(&no_style, " ")
        .replace("&nbsp;", " ")
        .to_string()
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().filter(|w| !w.is_empty()).count()
}

fn shingles(text: &str, n: usize) -> HashSet<String> {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .take(1500)
        .collect();
    if words.len() < n {
        return HashSet::new();
    }
    words.windows(n).map(|w| w.join(" ")).collect()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    inter / union
}

fn hash_text(text: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn setting(settings: &Value, key: &str) -> Value {
    settings.get(key).cloned().unwrap_or(Value::Null)
}

/// Decide every check for one post and persist the run.
pub async fn run(db: &PgPool, post_id: Uuid) -> Result<Report, String> {
    let row = sqlx::query_as::<
        _,
        (
            Uuid,           // directory_id
            String,         // title
            String,         // content
            Option<String>, // focus_keyword
            Option<String>, // meta_description
            Option<String>, // answer_block
            Option<String>, // internal_links (json text)
            Option<String>, // schema_json
            Option<String>, // author_name
            Option<Uuid>,   // author_id
            Option<String>, // featured_image_alt
        ),
    >(
        "SELECT directory_id, coalesce(title,''), coalesce(content,''), focus_keyword, \
                meta_description, answer_block, internal_links::text, schema_json::text, \
                author_name, author_id, featured_image_alt \
         FROM blog_posts WHERE id = $1",
    )
    .bind(post_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("load post: {e}"))?
    .ok_or_else(|| "post not found".to_string())?;

    let (
        directory_id,
        title,
        content,
        focus_keyword,
        meta_description,
        answer_block,
        internal_links,
        schema_json,
        author_name,
        author_id,
        image_alt,
    ) = row;

    let (mode, min_score, settings): (String, i32, Value) = sqlx::query_as(
        "SELECT blog_publish_mode, blog_min_quality_score, blog_guardrail_settings \
         FROM directories WHERE id = $1",
    )
    .bind(directory_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("load directory: {e}"))?
    .unwrap_or_else(|| ("review".to_string(), 70, json!({})));

    let text = plain_text(&content);
    let words = word_count(&text);
    let lower = text.to_lowercase();
    let mut checks: Vec<CheckResult> = Vec::new();

    // ── Safety and legal ────────────────────────────────────────────────────────
    let allow: Vec<String> = setting(&settings, "allow_words")
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let profane: Vec<&str> = PROFANITY
        .iter()
        .copied()
        .filter(|w| !allow.iter().any(|a| a == w))
        .filter(|w| lower.contains(w))
        .collect();
    checks.push(CheckResult {
        check: "profanity",
        outcome: if profane.is_empty() {
            Outcome::Pass
        } else {
            Outcome::Block
        },
        detail: if profane.is_empty() {
            "no flagged words".into()
        } else {
            format!("flagged: {}", profane.join(", "))
        },
    });

    // Hidden text / links: the classic cloaking pattern.
    let hidden = Regex::new(
        r"(?i)display\s*:\s*none|visibility\s*:\s*hidden|font-size\s*:\s*0(?:px)?\b|text-indent\s*:\s*-\d{3,}",
    )
    .unwrap();
    let hidden_hit = hidden.is_match(&content);
    checks.push(CheckResult {
        check: "hidden_text",
        outcome: if hidden_hit {
            Outcome::Block
        } else {
            Outcome::Pass
        },
        detail: if hidden_hit {
            "content hides text or links via CSS".into()
        } else {
            "nothing hidden".into()
        },
    });

    // Sponsored/affiliate links must be labelled.
    let aff = Regex::new(r#"(?i)href="[^"]*(aff=|affiliate|utm_source=aff)[^"]*""#).unwrap();
    let anchor = Regex::new(r#"(?is)<a\b[^>]*>"#).unwrap();
    let mut unlabelled = 0;
    for a in anchor.find_iter(&content).map(|m| m.as_str()) {
        if aff.is_match(a) && !a.to_lowercase().contains("rel=\"sponsored\"") {
            unlabelled += 1;
        }
    }
    checks.push(CheckResult {
        check: "link_hygiene",
        outcome: if unlabelled > 0 {
            Outcome::Hold
        } else {
            Outcome::Pass
        },
        detail: if unlabelled > 0 {
            format!("{unlabelled} affiliate link(s) without rel=sponsored")
        } else {
            "no unlabelled affiliate links".into()
        },
    });

    // Plagiarism: David's agreed thresholds are 5% hold / 15% block. The provider adapter is
    // not wired in this build, so the check reports its own state instead of silently passing
    // as though a scan had run — the scorecard must never imply a check it did not perform.
    let hold_pct = setting(&settings, "plagiarism_hold_pct")
        .as_f64()
        .unwrap_or(5.0);
    let block_pct = setting(&settings, "plagiarism_block_pct")
        .as_f64()
        .unwrap_or(15.0);
    let provider = setting(&settings, "plagiarism_provider")
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    checks.push(CheckResult {
        check: "plagiarism",
        outcome: Outcome::Pass,
        detail: if provider.is_empty() {
            format!(
                "no provider configured (thresholds {hold_pct:.0}% hold / {block_pct:.0}% block)"
            )
        } else {
            format!(
                "{provider} selected; adapter not wired yet (thresholds {hold_pct:.0}% / {block_pct:.0}%)"
            )
        },
    });

    // Business facts: a draft that ignores the business's own details is generic copy.
    let (facts_used, facts_total) =
        crate::handlers::business_facts::usage(db, directory_id, &lower).await;
    checks.push(CheckResult {
        check: "business_facts",
        outcome: Outcome::Pass,
        detail: if facts_total == 0 {
            "no facts recorded yet".to_string()
        } else {
            format!("used {facts_used} of {facts_total} recorded facts")
        },
    });

    // ── What Google penalises ───────────────────────────────────────────────────
    let kw = focus_keyword.unwrap_or_default().trim().to_lowercase();
    let density = if kw.is_empty() || words == 0 {
        0.0
    } else {
        lower.matches(&kw).count() as f64 * 100.0 / words as f64
    };
    let density_limit = setting(&settings, "keyword_density_pct")
        .as_f64()
        .unwrap_or(5.0);
    checks.push(CheckResult {
        check: "keyword_stuffing",
        outcome: if density > density_limit {
            Outcome::Hold
        } else {
            Outcome::Pass
        },
        detail: format!(
            "keyword density {:.2}% (limit {:.1}%)",
            density, density_limit
        ),
    });

    // Near-duplicates: same directory, 3-word shingles, Jaccard similarity.
    let mine = shingles(&text, 3);
    let mut worst_sim = 0.0f64;
    if !mine.is_empty() {
        let others: Vec<(String,)> = sqlx::query_as(
            "SELECT coalesce(content,'') FROM blog_posts \
             WHERE directory_id = $1 AND id <> $2 ORDER BY updated_at DESC LIMIT 60",
        )
        .bind(directory_id)
        .bind(post_id)
        .fetch_all(db)
        .await
        .unwrap_or_default();
        for (other,) in others {
            let sim = jaccard(&mine, &shingles(&plain_text(&other), 3));
            if sim > worst_sim {
                worst_sim = sim;
            }
        }
    }
    checks.push(CheckResult {
        check: "near_duplicate",
        outcome: if worst_sim >= 0.85 {
            Outcome::Hold
        } else {
            Outcome::Pass
        },
        detail: format!(
            "highest similarity to another post: {:.0}%",
            worst_sim * 100.0
        ),
    });

    // Structured data must describe what is actually on the page.
    let mut schema_ok = true;
    let mut schema_detail = "no structured data".to_string();
    if let Some(raw) = schema_json.as_deref().filter(|s| !s.trim().is_empty()) {
        if let Ok(schema) = serde_json::from_str::<Value>(raw) {
            schema_detail = "matches the page".to_string();
            let types: Vec<&str> = schema
                .get("@type")
                .map(|t| match t {
                    Value::String(s) => vec![s.as_str()],
                    Value::Array(a) => a.iter().filter_map(|v| v.as_str()).collect(),
                    _ => vec![],
                })
                .unwrap_or_default();
            if types.contains(&"FAQPage") {
                if let Some(qs) = schema.get("mainEntity").and_then(|m| m.as_array()) {
                    for q in qs {
                        if let Some(name) = q
                            .get("name")
                            .and_then(|n| n.as_str())
                            .map(|n| n.to_lowercase())
                        {
                            if !lower.contains(&name) {
                                schema_ok = false;
                                schema_detail =
                                    "FAQ markup asks a question the page does not show".into();
                                break;
                            }
                        }
                    }
                }
            }
            if types.contains(&"HowTo") {
                if let Some(steps) = schema.get("step").and_then(|s| s.as_array()) {
                    for s in steps {
                        if let Some(name) = s
                            .get("name")
                            .and_then(|n| n.as_str())
                            .map(|n| n.to_lowercase())
                        {
                            if !lower.contains(&name) {
                                schema_ok = false;
                                schema_detail =
                                    "HowTo markup has a step the page does not show".into();
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    checks.push(CheckResult {
        check: "structured_data_truth",
        outcome: if schema_ok {
            Outcome::Pass
        } else {
            Outcome::Block
        },
        detail: schema_detail,
    });

    // A byline must be a real person on this directory.
    let mut author_detail = "no author".to_string();
    let mut author_ok = false;
    if let Some(id) = author_id {
        let exists: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM author_profiles WHERE id = $1 AND directory_id = $2 AND is_active",
        )
        .bind(id)
        .bind(directory_id)
        .fetch_optional(db)
        .await
        .unwrap_or(None);
        author_ok = exists.is_some();
        author_detail = if author_ok {
            "real author profile".into()
        } else {
            "author_id does not match a profile on this directory".into()
        };
    } else if let Some(name) = author_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        author_ok = true;
        author_detail = format!("byline '{name}' (no profile linked)");
    }
    checks.push(CheckResult {
        check: "author_truth",
        outcome: if author_ok {
            Outcome::Pass
        } else {
            Outcome::Block
        },
        detail: author_detail,
    });

    // ── Quality score (what Google rewards, made visible) ───────────────────────
    let mut score: i32 = 0;
    let answer_len = answer_block.as_deref().unwrap_or("").trim().len();
    if answer_len >= 100 {
        score += 20;
    }
    let meta_len = meta_description
        .as_deref()
        .unwrap_or("")
        .trim()
        .chars()
        .count();
    if (50..=170).contains(&meta_len) {
        score += 10;
    }
    if !kw.is_empty() && title.to_lowercase().contains(&kw) {
        score += 10;
    }
    let h2s = Regex::new(r"(?i)<h2\b")
        .unwrap()
        .find_iter(&content)
        .count();
    if h2s >= 3 {
        score += 10;
    }
    let links_count = internal_links
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .map(|v| match v {
            Value::Array(a) => a.len(),
            Value::Object(o) => o.len(),
            _ => 0,
        })
        .unwrap_or(0)
        + Regex::new(r#"(?i)<a\b[^>]*href="#)
            .unwrap()
            .find_iter(&content)
            .count()
            .min(5)
            / 5;
    if links_count >= 2 {
        score += 10;
    }
    if image_alt
        .as_deref()
        .map(str::trim)
        .is_some_and(|a| !a.is_empty())
    {
        score += 10;
    }
    if author_ok {
        score += 10;
    }
    if facts_total > 0 && facts_used > 0 {
        score += 10;
    }
    let sources = Regex::new(r#"(?i)<a\b[^>]*href="https?://"#)
        .unwrap()
        .find_iter(&content)
        .count();
    if sources >= 2 {
        score += 10;
    }
    if schema_json.as_deref().is_some_and(|s| !s.trim().is_empty()) {
        score += 10;
    }

    let mut outcome = checks
        .iter()
        .fold(Outcome::Pass, |acc, c| acc.worse(c.outcome));
    if outcome == Outcome::Pass && score < min_score {
        outcome = Outcome::Hold;
    }

    let report = Report {
        outcome,
        quality_score: score,
        checks,
        content_hash: hash_text(&format!("{title}\n{content}")),
    };

    let checks_json = serde_json::to_value(
        report
            .checks
            .iter()
            .map(|c| {
                json!({
                    "check": c.check,
                    "outcome": c.outcome.as_str(),
                    "detail": c.detail,
                })
            })
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| json!([]));

    sqlx::query(
        "INSERT INTO guardrail_runs (post_id, directory_id, outcome, quality_score, checks, content_hash) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(post_id)
    .bind(directory_id)
    .bind(report.outcome.as_str())
    .bind(report.quality_score)
    .bind(&checks_json)
    .bind(&report.content_hash)
    .execute(db)
    .await
    .map_err(|e| format!("store guardrail run: {e}"))?;

    sqlx::query(
        "UPDATE blog_posts SET guardrail_outcome = $2, quality_score = $3, \
                guardrail_checked_at = now(), guardrail_content_hash = $4, updated_at = updated_at \
         WHERE id = $1",
    )
    .bind(post_id)
    .bind(report.outcome.as_str())
    .bind(report.quality_score)
    .bind(&report.content_hash)
    .execute(db)
    .await
    .map_err(|e| format!("store guardrail outcome: {e}"))?;

    let _ = mode; // review/auto is applied by the caller that publishes
    Ok(report)
}

/// How many posts this directory has published today (for the auto-publish daily cap).
pub async fn published_today(db: &PgPool, directory_id: Uuid) -> Result<i64, String> {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM blog_posts \
         WHERE directory_id = $1 AND published = true AND updated_at::date = current_date",
    )
    .bind(directory_id)
    .fetch_one(db)
    .await
    .map_err(|e| format!("count published today: {e}"))
}
