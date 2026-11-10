//! What a container or cluster action could do, before it runs.
//!
//! These are not command-line classifications: by the time one of these is asked, the action is
//! already a fixed verb with a fixed argv. What is being decided is whether the interface may send
//! it without an explicit acknowledgement — the same gate batch execution uses.
//!
//! The terminal is deliberately outside this gate. `exec` starts a shell a person types into; the
//! typing is the decision. A button that calls `rm` on a container is a different thing.

use sevenshell_types::{RiskAssessment, RiskLevel, RiskReason};

/// Classifies one container operation.
///
/// Unknown verbs are `Caution`, never `Safe`: a caller this table has not learned about should be
/// looked at, not waved through.
pub fn container_action(action: &str) -> RiskAssessment {
    let action = action.trim().to_ascii_lowercase();
    let verdict = match action.as_str() {
        "list" | "inspect" | "logs" | "stats" | "images" | "namespaces" | "info" | "exec" => {
            return RiskAssessment::safe();
        }
        "start" | "restart" | "pause" | "unpause" | "rename" | "create" | "tag" | "pull" | "push" | "login"
        | "load" | "commit" => ("container.change", "risk.container.change", RiskLevel::Caution),
        "stop" | "kill" => ("container.stop", "risk.container.stop", RiskLevel::Destructive),
        "remove" | "image-remove" | "prune" => ("container.remove", "risk.container.remove", RiskLevel::Destructive),
        _ => ("container.unknown", "risk.container.unknown", RiskLevel::Caution),
    };
    RiskAssessment::from_reasons(vec![RiskReason::new(verdict.0, verdict.1, verdict.2)])
}

/// Classifies one generic API request.
///
/// The path is what carries the meaning: a `DELETE` of a Namespace can take every object in it
/// with it, an eviction is how a node drain removes workloads, and everything else that writes is
/// a change somebody should see named before it happens.
pub fn kubernetes_request(method: &str, path: &str) -> RiskAssessment {
    let method = method.trim().to_ascii_uppercase();
    let path = path.trim();
    let verdict = match method.as_str() {
        "GET" | "HEAD" => return RiskAssessment::safe(),
        "DELETE" if deletes_a_namespace(path) => (
            "k8s.namespace-delete",
            "risk.k8s.namespaceDelete",
            RiskLevel::Catastrophic,
        ),
        "DELETE" => ("k8s.delete", "risk.k8s.delete", RiskLevel::Destructive),
        "POST" if path.ends_with("/eviction") => ("k8s.evict", "risk.k8s.evict", RiskLevel::Destructive),
        "POST" | "PUT" | "PATCH" => ("k8s.write", "risk.k8s.write", RiskLevel::Caution),
        _ => ("k8s.unknown", "risk.k8s.unknown", RiskLevel::Caution),
    };
    RiskAssessment::from_reasons(vec![RiskReason::new(verdict.0, verdict.1, verdict.2)])
}

/// Whether a path names the namespace object itself, rather than something inside it.
///
/// `/api/v1/namespaces/prod` is the namespace; `/api/v1/namespaces/prod/pods/web` is a pod. The
/// difference is the whole verdict: one takes every object in it, the other takes one.
fn deletes_a_namespace(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').filter(|segment| !segment.is_empty()).collect();
    matches!(segments.as_slice(), ["api", "v1", "namespaces", _])
        || matches!(segments.as_slice(), ["apis", _, _, "namespaces", _])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_a_container_never_asks_for_approval() {
        for action in ["list", "inspect", "logs", "stats", "images", "namespaces", "info"] {
            let verdict = container_action(action);
            assert_eq!(verdict.level, RiskLevel::Safe, "{action} should not be gated");
            assert!(!verdict.needs_approval());
        }
    }

    #[test]
    fn typing_in_a_container_is_not_gated() {
        assert_eq!(container_action("exec").level, RiskLevel::Safe);
    }

    #[test]
    fn stopping_and_removing_need_an_acknowledgement() {
        assert_eq!(container_action("stop").level, RiskLevel::Destructive);
        assert_eq!(container_action("remove").level, RiskLevel::Destructive);
        assert_eq!(container_action("prune").level, RiskLevel::Destructive);
        assert!(container_action("remove").needs_approval());
    }

    #[test]
    fn an_unknown_container_verb_is_caution_not_safe() {
        assert_eq!(container_action("teleport").level, RiskLevel::Caution);
    }

    #[test]
    fn reading_the_cluster_is_safe_and_writing_is_not() {
        assert_eq!(kubernetes_request("GET", "/api/v1/pods").level, RiskLevel::Safe);
        assert_eq!(
            kubernetes_request("PATCH", "/api/v1/nodes/lab").level,
            RiskLevel::Caution
        );
        assert_eq!(
            kubernetes_request("DELETE", "/api/v1/namespaces/prod/pods/web").level,
            RiskLevel::Destructive
        );
        assert_eq!(
            kubernetes_request("DELETE", "/apis/apps/v1/namespaces/prod/deployments/web").level,
            RiskLevel::Destructive
        );
        assert_eq!(
            kubernetes_request("DELETE", "/api/v1/namespaces/prod").level,
            RiskLevel::Catastrophic
        );
        assert_eq!(
            kubernetes_request("POST", "/api/v1/namespaces/prod/pods/web/eviction").level,
            RiskLevel::Destructive
        );
    }

    #[test]
    fn an_unknown_method_is_caution_not_safe() {
        assert_eq!(kubernetes_request("PROPFIND", "/api/v1/pods").level, RiskLevel::Caution);
    }
}
