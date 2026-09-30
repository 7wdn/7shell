//! The classifier's behaviour, locked case by case.
//!
//! The corpus exists because this code is quick to loosen and slow to notice: a rule that stops
//! firing still compiles, still returns a level, and only shows itself when someone runs the wrong
//! thing on ten hosts at once. Every case here is a decision that has to be changed on purpose.
//!
//! ```text
//! cargo test -p sevenshell-policy --test corpus
//! ```

use serde::Deserialize;
use sevenshell_policy::{RiskLevel, classify};

/// One line of the corpus: a command, the level it must have, and the rules that must have fired.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Case {
    command: String,
    level: RiskLevel,
    /// Rules that must be among the reasons. Containment rather than equality: adding a rule that
    /// also fires is a smaller change than removing one that used to, and the corpus records the
    /// ones that matter instead of freezing the exact set.
    reasons: Vec<String>,
    #[serde(default)]
    note: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Corpus {
    note: String,
    cases: Vec<Case>,
}

fn corpus() -> Corpus {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/command-risk-corpus.json");
    let raw = std::fs::read_to_string(path).expect("the corpus ships with the crate");
    serde_json::from_str(&raw).expect("the corpus is valid JSON in the documented shape")
}

#[test]
fn every_case_has_the_level_it_records() {
    let corpus = corpus();
    assert!(
        corpus.cases.len() >= 40,
        "a corpus this small proves nothing: {}",
        corpus.note
    );

    let mut wrong = Vec::new();
    for case in &corpus.cases {
        let assessment = classify(&case.command);
        if assessment.level != case.level {
            let fired: Vec<&str> = assessment
                .reasons
                .iter()
                .map(|reason| reason.rule_id.as_str())
                .collect();
            wrong.push(format!(
                "  {:?}\n    expected {:?}, got {:?} (fired: {:?}){}",
                case.command,
                case.level,
                assessment.level,
                fired,
                if case.note.is_empty() {
                    String::new()
                } else {
                    format!("\n    note: {}", case.note)
                },
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "the classifier disagrees with the corpus in {} case(s):\n{}\n\
         Fix the rule or the corpus — but decide which one is right first.",
        wrong.len(),
        wrong.join("\n"),
    );
}

#[test]
fn every_case_has_the_reasons_it_records() {
    let mut wrong = Vec::new();

    for case in &corpus().cases {
        let assessment = classify(&case.command);
        let fired: Vec<&str> = assessment
            .reasons
            .iter()
            .map(|reason| reason.rule_id.as_str())
            .collect();
        for expected in &case.reasons {
            if !fired.contains(&expected.as_str()) {
                wrong.push(format!(
                    "  {:?} did not fire {expected:?} (fired: {fired:?})",
                    case.command
                ));
            }
        }
    }

    assert!(wrong.is_empty(), "rules went quiet:\n{}", wrong.join("\n"));
}

/// A reason's message key is what the interface translates, so a rule with no key would show up as
/// a raw identifier in the warning — or as nothing at all.
#[test]
fn every_reason_is_translatable() {
    for case in &corpus().cases {
        for reason in classify(&case.command).reasons {
            assert!(
                reason.label_key.starts_with("risk."),
                "rule {} has the message key {:?}, which is not under risk.*",
                reason.rule_id,
                reason.label_key,
            );
            assert!(!reason.rule_id.is_empty());
        }
    }
}
