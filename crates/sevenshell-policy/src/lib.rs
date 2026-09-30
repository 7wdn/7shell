//! Deciding what is allowed, and how dangerous it is.
//!
//! The project's seventh invariant says every privileged or destructive action has a policy
//! decision behind it. This crate is where those decisions are made, starting with the one that
//! matters most for an operations console: **how dangerous is this command?**
//!
//! The classifier is deliberately explainable. It never answers "this looks risky": it names the
//! rules that fired, each with a message key, so the interface can say *why* and a person can
//! disagree with a specific rule instead of with a black box.

pub mod command_risk;

pub use command_risk::{RiskAssessment, RiskLevel, RiskReason, classify};
