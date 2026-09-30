//! Running one command on many hosts, and what came back from each.
//!
//! A batch is different from a terminal in the one way that shapes all of these types: it answers
//! **per host**. One host being unreachable, slow, or gone from the vault does not stop the others,
//! and the person reading the result needs to tell those apart — which is why a host's failure is
//! data in the result rather than an error that replaces it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{KnownHostEntry, PresentedHostKey, RiskAssessment};

/// What to run, and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchRequest {
    /// The saved sessions to run on, in the order the results should come back.
    pub session_ids: Vec<String>,
    /// One command line, run by the server's shell on every host.
    pub command: String,
    /// How long one host may take. Clamped to a sane range by the caller of the transport, so a
    /// zero or an absurd number is corrected rather than obeyed.
    pub timeout_ms: u64,
    /// How many hosts to work on at once. Also clamped.
    pub parallel: u16,
    /// Set once somebody has seen what the command would do — see
    /// [`BatchOutcome::ApprovalRequired`]. Without it, a command that needs approval comes back
    /// with its verdict instead of running.
    #[serde(default)]
    pub acknowledged: bool,
}

/// What a batch did, or what it is waiting for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum BatchOutcome {
    /// The host key of this host has never been seen. Nothing was run anywhere, and the same
    /// request after trusting the key will try again.
    HostKeyUnknown { key: PresentedHostKey },
    /// The presented key differs from the trusted one. A hard stop, not a prompt.
    HostKeyMismatch {
        key: PresentedHostKey,
        trusted: KnownHostEntry,
    },
    /// The command is dangerous enough that it waits for a person.
    ///
    /// Returned instead of running, with the verdict attached, so whatever asked can show the
    /// rules that fired and ask again with `acknowledged: true`. The classifier is not weakened by
    /// acknowledgement: the flag records that somebody was shown the reasons, not that the
    /// command became safe.
    ApprovalRequired { risk: RiskAssessment },
    /// Every host has an answer.
    Finished {
        /// The command that was run, so a result shown later still says what it was.
        command: String,
        /// The verdict for that command: one verdict per run, not per host.
        risk: RiskAssessment,
        /// One row per host, in the order the request listed them — always as long as the request,
        /// including the hosts that never got a connection.
        results: Vec<BatchHostResult>,
        /// How long the whole run took, which is not the sum of the hosts once they run in
        /// parallel.
        duration_ms: u64,
    },
}

/// What one host has to say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchHostResult {
    pub session_id: String,
    /// The name as it was saved, so a result is readable without looking the id up.
    pub session_name: String,
    pub outcome: BatchHostOutcome,
    pub duration_ms: u64,
}

/// One host's answer to the command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum BatchHostOutcome {
    /// The command ran, and this is what it said.
    Ran {
        /// The status the server reported. A missing one is not success: a command killed by a
        /// signal, or one that was cut off, leaves this empty.
        exit_code: Option<u32>,
        stdout: String,
        stderr: String,
    },
    /// The command never ran on this host, and this is why.
    Failed { error: HostFailure },
}

/// Why one host has nothing to show.
///
/// A copy of the parts of the error envelope a person reads — the code, the message key and its
/// parameters, and the scrubbed detail — rather than the envelope itself. The two crates are
/// peers, so one cannot name the other's type; the conversion lives in the layer that has both,
/// and a test there proves no field is dropped on the way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostFailure {
    /// The stable code from the error catalog, e.g. `SSH-TIMEOUT`.
    pub code: String,
    /// The catalog's message key, which the interface translates.
    pub message_key: String,
    /// The values that message needs, such as how many seconds passed.
    pub message_params: BTreeMap<String, String>,
    /// The scrubbed explanation, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
