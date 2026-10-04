//! The ZaarCash float rule — three independent conditions, HOLD if any fails.
//!
//! David, 2026-10-04: *"You come up with best industry practice float rule. But again that
//! should be able to be configured also by the Admin in the admin panel."*
//!
//! Before this, the float was ONE flat comparison (`available >= minimum_float`). One number
//! cannot adapt: a programme with a hundred points outstanding and one with a million need the
//! same *relationship* held, not the same dollar figure. So the float is judged on three
//! independent conditions:
//!
//! | Rule     | Condition                                                | Why |
//! |----------|----------------------------------------------------------|-----|
//! | Coverage | `available >= coverage_pct% x outstanding liability`      | accounting view — never redeem points nobody funded |
//! | Burn     | `available >= burn_months x trailing-30-day redemptions`   | operational view — a programme can be fully covered on paper and still fail when several members redeem at once |
//! | Floor    | `available >= minimum_float`                              | risk view — a brand-new programme has near-zero liability *and* near-zero burn, so it would pass the other two while holding nothing |
//!
//! Action on breach: **hold**. Points stay valid, nothing is confiscated, redemptions resume on
//! top-up. A malformed setting falls back to the conservative default — a bad value fails safe,
//! never open.
//!
//! All three thresholds are admin settings (`point_treasury.float_coverage_pct`,
//! `point_treasury.float_burn_months`, `point_treasury.minimum_float`), editable from the
//! admin panel's Clearinghouse card. Nothing here is hardcoded: the defaults are only what an
//! admin gets before they touch anything.

use rust_decimal::prelude::FromStr;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{json, Value};

/// Coverage default: hold cash for 100% of the points members are holding.
pub const DEFAULT_COVERAGE_PCT: i32 = 100;
/// Burn default: hold one month of redemptions on top.
pub const DEFAULT_BURN_MONTHS: i64 = 1;
/// Floor default: the smallest float a programme may run on, in dollars.
pub const DEFAULT_FLOAT_FLOOR: i64 = 100;
/// Coverage is a percentage of liability; above 100 is not a rule an admin can reach.
pub const MAX_COVERAGE_PCT: i32 = 100;
/// Seasonal programmes (holiday/event-driven redemption) may want 2–3 months; 12 is the ceiling.
pub const MAX_BURN_MONTHS: i64 = 12;
/// A floor above a million dollars is a typo, not a policy.
pub const MAX_FLOAT_FLOOR: i64 = 1_000_000;

/// The three thresholds the float is measured against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloatRule {
    /// Percentage of outstanding liability that must be on hand (100 = fully covered).
    pub coverage_pct: i32,
    /// Months of trailing redemption volume that must be on hand.
    pub burn_months: Decimal,
    /// Hard minimum, in dollars. `point_treasury.minimum_float` IS this value.
    pub floor: Decimal,
}

impl Default for FloatRule {
    fn default() -> Self {
        Self {
            coverage_pct: DEFAULT_COVERAGE_PCT,
            burn_months: Decimal::from(DEFAULT_BURN_MONTHS),
            floor: Decimal::from(DEFAULT_FLOAT_FLOOR),
        }
    }
}

/// One rule's outcome, with the actual numbers in a sentence a business owner can read —
/// "breach" teaches a business owner nothing.
#[derive(Debug, Clone, Serialize)]
pub struct RuleOutcome {
    /// Stable key: `coverage` | `burn` | `floor`.
    pub key: &'static str,
    /// Human label for the panel.
    pub label: &'static str,
    /// Dollars the rule demands.
    pub required: Decimal,
    /// Dollars on hand (same for every rule; repeated so the panel can render a row alone).
    pub available: Decimal,
    pub passed: bool,
    /// Plain English, naming the actual numbers.
    pub sentence: String,
}

/// The verdict for one network.
#[derive(Debug, Clone, Serialize)]
pub struct FloatVerdict {
    pub safe: bool,
    pub available: Decimal,
    pub outstanding_liability: Decimal,
    pub coverage_pct: i32,
    pub burn_months: Decimal,
    pub floor: Decimal,
    /// Trailing-30-day redemption volume in dollars; `None` = nothing to size a month against.
    pub monthly_burn: Option<Decimal>,
    pub outcomes: Vec<RuleOutcome>,
}

fn money(d: Decimal) -> String {
    format!("{:.2}", d)
}

/// A setting the admin typed, read back defensively. Anything that is not a number in range
/// falls back to the conservative default; a bad value fails safe, never open.
pub fn parse_coverage(raw: Option<&str>) -> i32 {
    raw.and_then(|s| s.trim().parse::<i32>().ok())
        .filter(|v| (0..=MAX_COVERAGE_PCT).contains(v))
        .unwrap_or(DEFAULT_COVERAGE_PCT)
}

pub fn parse_burn_months(raw: Option<&str>) -> Decimal {
    raw.and_then(|s| Decimal::from_str(s.trim()).ok())
        .filter(|v| *v >= Decimal::ZERO && *v <= Decimal::from(MAX_BURN_MONTHS))
        .unwrap_or_else(|| Decimal::from(DEFAULT_BURN_MONTHS))
}

pub fn parse_floor(raw: Option<&str>) -> Decimal {
    raw.and_then(|s| Decimal::from_str(s.trim()).ok())
        .filter(|v| *v >= Decimal::ZERO && *v <= Decimal::from(MAX_FLOAT_FLOOR))
        .unwrap_or_else(|| Decimal::from(DEFAULT_FLOAT_FLOOR))
}

/// Build the rule from what is stored. `floor` arrives as a typed column (`minimum_float`); the
/// other two are parsed from their stored text so a value written by hand in SQL still fails
/// safe. A negative floor is treated as the default, not as "no floor".
pub fn rule_from_settings(coverage: Option<&str>, burn: Option<&str>, floor: Decimal) -> FloatRule {
    let floor = if floor >= Decimal::ZERO && floor <= Decimal::from(MAX_FLOAT_FLOOR) {
        floor
    } else {
        Decimal::from(DEFAULT_FLOAT_FLOOR)
    };
    FloatRule {
        coverage_pct: parse_coverage(coverage),
        burn_months: parse_burn_months(burn),
        floor,
    }
}

/// Judge a network's float.
///
/// * a negative `outstanding_liability` (reachable with test data / a rate change) counts as zero,
///   so the coverage rule never demands a positive float from a negative liability;
/// * `monthly_burn` `None` means the burn rule cannot fail — there is nothing to size a month
///   against (a brand-new programme has no history), and the floor is what protects it.
pub fn evaluate(
    available: Decimal,
    outstanding_liability: Decimal,
    monthly_burn: Option<Decimal>,
    rule: &FloatRule,
) -> FloatVerdict {
    let liability = outstanding_liability.max(Decimal::ZERO);

    let coverage_required = liability * Decimal::from(rule.coverage_pct) / Decimal::ONE_HUNDRED;
    let coverage_passed = available >= coverage_required;
    let coverage_sentence = if coverage_passed {
        format!(
            "You hold ${} against ${} of ZaarCash in your members' hands — covered at {}%.",
            money(available),
            money(liability),
            rule.coverage_pct
        )
    } else {
        format!(
            "You hold ${} but your members are holding ZaarCash worth ${}; the rule needs ${} on hand ({}%).",
            money(available),
            money(liability),
            money(coverage_required),
            rule.coverage_pct
        )
    };

    let (burn_required, burn_passed, burn_sentence) = match monthly_burn {
        None => (
            Decimal::ZERO,
            true,
            "No redemptions in the last 30 days, so there is no burn to size against.".to_string(),
        ),
        Some(burn) => {
            let burn = burn.max(Decimal::ZERO);
            let required = burn * rule.burn_months;
            let passed = available >= required;
            let sentence = if passed {
                format!(
                    "Redemptions ran at about ${} a month; you hold ${} — {} month(s) of cover.",
                    money(burn),
                    money(available),
                    rule.burn_months
                )
            } else {
                format!(
                    "Redemptions ran at about ${} over the last 30 days; {} month(s) of cover needs ${} but you hold ${}.",
                    money(burn),
                    rule.burn_months,
                    money(required),
                    money(available)
                )
            };
            (required, passed, sentence)
        }
    };

    let floor_required = rule.floor.max(Decimal::ZERO);
    let floor_passed = available >= floor_required;
    let floor_sentence = if floor_passed {
        format!(
            "You hold ${}, at or above the ${} minimum float.",
            money(available),
            money(floor_required)
        )
    } else {
        format!(
            "The programme requires a minimum float of ${}; you hold ${}.",
            money(floor_required),
            money(available)
        )
    };

    let outcomes = vec![
        RuleOutcome {
            key: "coverage",
            label: "Coverage — cash behind the ZaarCash members hold",
            required: coverage_required,
            available,
            passed: coverage_passed,
            sentence: coverage_sentence,
        },
        RuleOutcome {
            key: "burn",
            label: "Burn — cash for a month of redemptions",
            required: burn_required,
            available,
            passed: burn_passed,
            sentence: burn_sentence,
        },
        RuleOutcome {
            key: "floor",
            label: "Floor — the minimum float",
            required: floor_required,
            available,
            passed: floor_passed,
            sentence: floor_sentence,
        },
    ];

    let safe = outcomes.iter().all(|o| o.passed);

    FloatVerdict {
        safe,
        available,
        outstanding_liability: liability,
        coverage_pct: rule.coverage_pct,
        burn_months: rule.burn_months,
        floor: floor_required,
        monthly_burn,
        outcomes,
    }
}

impl FloatVerdict {
    /// Keys of the rules that failed, in rule order.
    pub fn failures(&self) -> Vec<&str> {
        self.outcomes
            .iter()
            .filter(|o| !o.passed)
            .map(|o| o.key)
            .collect()
    }

    /// Sentences of the rules that failed.
    pub fn failure_sentences(&self) -> Vec<&str> {
        self.outcomes
            .iter()
            .filter(|o| !o.passed)
            .map(|o| o.sentence.as_str())
            .collect()
    }

    /// One line: healthy, or what is short and by how much.
    pub fn sentence(&self) -> String {
        if self.safe {
            format!(
                "Float is healthy: ${} on hand meets all three checks.",
                money(self.available)
            )
        } else {
            format!(
                "Redemptions are on hold: {}",
                self.failure_sentences().join(" ")
            )
        }
    }

    /// The payload the admin panel renders. The console shows the SERVER's verdict, never its own
    /// arithmetic — otherwise it can claim a behaviour the code does not perform.
    pub fn to_json(&self) -> Value {
        let conditions: Vec<Value> = self
            .outcomes
            .iter()
            .map(|o| {
                json!({
                    "key": o.key,
                    "label": o.label,
                    "required": money(o.required),
                    "available": money(o.available),
                    "passed": o.passed,
                    "sentence": o.sentence,
                })
            })
            .collect();
        json!({
            "safe": self.safe,
            "available": money(self.available),
            "outstanding_liability": money(self.outstanding_liability),
            "coverage_pct": self.coverage_pct,
            "burn_months": format!("{}", self.burn_months),
            "minimum_float": money(self.floor),
            "monthly_burn": self.monthly_burn.map(money),
            "conditions": conditions,
            "failures": self.failures(),
            "sentence": self.sentence(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn verdict(
        available: &str,
        liability: &str,
        burn: Option<&str>,
        rule: &FloatRule,
    ) -> FloatVerdict {
        evaluate(d(available), d(liability), burn.map(d), rule)
    }

    #[test]
    fn all_three_pass_when_funded() {
        let v = verdict("500", "100", Some("50"), &FloatRule::default());
        assert!(v.safe, "{}", v.sentence());
        assert!(v.failures().is_empty());
    }

    #[test]
    fn coverage_fails_when_liability_is_not_funded() {
        let v = verdict("40", "100", None, &FloatRule::default());
        assert!(!v.safe);
        assert!(v.failures().contains(&"coverage"));
        assert!(v.sentence().contains("your members are holding"));
    }

    #[test]
    fn burn_fails_even_when_coverage_passes() {
        // Fully covered on paper (liability 0) but a month of redemptions is not on hand.
        let v = verdict("40", "0", Some("100"), &FloatRule::default());
        assert!(
            v.outcomes
                .iter()
                .find(|o| o.key == "coverage")
                .unwrap()
                .passed
        );
        assert!(v.failures().contains(&"burn"));
    }

    #[test]
    fn burn_cannot_fail_without_history() {
        let v = verdict("500", "0", None, &FloatRule::default());
        assert!(v.outcomes.iter().find(|o| o.key == "burn").unwrap().passed);
    }

    #[test]
    fn floor_fails_for_a_brand_new_programme() {
        // No liability, no burn — only the floor stands between an empty treasury and redemptions.
        let v = verdict("0", "0", None, &FloatRule::default());
        assert!(!v.safe);
        assert_eq!(v.failures(), vec!["floor"]);
    }

    #[test]
    fn negative_liability_is_treated_as_zero() {
        let v = verdict(
            "0",
            "-500",
            None,
            &FloatRule {
                floor: Decimal::ZERO,
                ..FloatRule::default()
            },
        );
        assert!(v.safe, "{}", v.sentence());
    }

    #[test]
    fn malformed_settings_fall_back_to_the_safe_default() {
        assert_eq!(parse_coverage(Some("abc")), DEFAULT_COVERAGE_PCT);
        assert_eq!(parse_coverage(Some("")), DEFAULT_COVERAGE_PCT);
        assert_eq!(parse_coverage(Some("-10")), DEFAULT_COVERAGE_PCT);
        assert_eq!(parse_coverage(Some("5000")), DEFAULT_COVERAGE_PCT);
        assert_eq!(parse_coverage(None), DEFAULT_COVERAGE_PCT);
        assert_eq!(parse_burn_months(Some("nope")), Decimal::from(1));
        assert_eq!(parse_burn_months(Some("1.5")), d("1.5"));
        assert_eq!(parse_burn_months(Some("99")), Decimal::from(1));
        assert_eq!(parse_floor(Some("-3")), Decimal::from(DEFAULT_FLOAT_FLOOR));
        // A negative floor cannot be stored as "no floor" through this path.
        let r = rule_from_settings(None, None, d("-1"));
        assert_eq!(r.floor, Decimal::from(DEFAULT_FLOAT_FLOOR));
    }

    #[test]
    fn admin_can_turn_each_rule_off_with_a_zero() {
        let r = FloatRule {
            coverage_pct: 0,
            burn_months: Decimal::ZERO,
            floor: Decimal::ZERO,
        };
        let v = verdict("0", "1000", Some("500"), &r);
        assert!(v.safe, "{}", v.sentence());
    }

    #[test]
    fn coverage_is_a_percentage_of_liability() {
        let r = FloatRule {
            coverage_pct: 50,
            ..FloatRule::default()
        };
        let v = verdict("50", "100", None, &r);
        assert!(
            v.outcomes
                .iter()
                .find(|o| o.key == "coverage")
                .unwrap()
                .passed
        );
        let v = verdict("49.99", "100", None, &r);
        assert!(v.failures().contains(&"coverage"));
    }

    #[test]
    fn json_names_the_numbers() {
        let v = verdict("40", "100", None, &FloatRule::default());
        let j = v.to_json();
        assert_eq!(j["safe"], json!(false));
        assert_eq!(j["available"], json!("40.00"));
        assert_eq!(j["minimum_float"], json!("100.00"));
        assert_eq!(j["conditions"].as_array().unwrap().len(), 3);
        assert_eq!(j["conditions"][0]["required"], json!("100.00"));
        assert!(j["sentence"].as_str().unwrap().contains("on hold"));
    }
}
