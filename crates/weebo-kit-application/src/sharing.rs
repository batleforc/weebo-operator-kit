//! Whether an object shares itself (its `spec.sharedWith`) with a CR of
//! another namespace — or with cluster-scoped CRs. The same consent for a
//! local instance used from elsewhere and for a CR referenced across
//! namespaces.

use std::collections::BTreeMap;

use weebo_kit_api::SharedWith;

/// `shared` lets a CR of `cr_namespace` (`None`: cluster-scoped) use the
/// object: its namespace is listed, or matched by the selector (given the
/// namespace's labels — none known: no match), or cluster-scoped CRs are
/// accepted. Unset: nobody outside the object's own namespace.
pub fn shares_with(
    shared: Option<&SharedWith>,
    cr_namespace: Option<&str>,
    cr_namespace_labels: Option<&BTreeMap<String, String>>,
) -> bool {
    let Some(shared) = shared else {
        return false;
    };
    match cr_namespace {
        None => shared.cluster_scoped,
        Some(ns) => {
            shared.namespaces.iter().any(|n| n == ns)
                || shared.namespace_selector.as_ref().is_some_and(|sel| {
                    cr_namespace_labels.is_some_and(|labels| selector_matches(sel, labels))
                })
        }
    }
}

/// `selector` (a Kubernetes `LabelSelector`) matches `labels`. An
/// unknown operator matches nothing: a sharing rule never widens by
/// mistake.
fn selector_matches(
    selector: &k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector,
    labels: &BTreeMap<String, String>,
) -> bool {
    let labels_ok = selector
        .match_labels
        .iter()
        .flatten()
        .all(|(k, v)| labels.get(k) == Some(v));
    let expressions_ok = selector.match_expressions.iter().flatten().all(|e| {
        let values = e.values.as_deref().unwrap_or_default();
        let value = labels.get(&e.key);
        match e.operator.as_str() {
            "In" => value.is_some_and(|v| values.contains(v)),
            "NotIn" => value.is_none_or(|v| !values.contains(v)),
            "Exists" => value.is_some(),
            "DoesNotExist" => value.is_none(),
            _ => false,
        }
    });
    labels_ok && expressions_ok
}

#[cfg(test)]
mod tests {
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, LabelSelectorRequirement};

    use super::*;

    #[test]
    fn listed_selected_or_cluster_scoped() {
        let shared = SharedWith {
            namespaces: vec!["team-b".into()],
            namespace_selector: Some(LabelSelector {
                match_expressions: Some(vec![LabelSelectorRequirement {
                    key: "tier".into(),
                    operator: "NotIn".into(),
                    values: Some(vec!["test".into()]),
                }]),
                ..Default::default()
            }),
            cluster_scoped: false,
        };
        assert!(shares_with(Some(&shared), Some("team-b"), None));
        assert!(!shares_with(Some(&shared), Some("team-c"), None));
        let front = BTreeMap::from([("tier".to_string(), "front".to_string())]);
        assert!(shares_with(Some(&shared), Some("team-c"), Some(&front)));
        let test = BTreeMap::from([("tier".to_string(), "test".to_string())]);
        assert!(!shares_with(Some(&shared), Some("team-c"), Some(&test)));
        assert!(!shares_with(Some(&shared), None, None));
        assert!(!shares_with(None, Some("team-b"), None));
    }

    #[test]
    fn an_unknown_operator_matches_nothing() {
        let selector = LabelSelector {
            match_expressions: Some(vec![LabelSelectorRequirement {
                key: "k".into(),
                operator: "Bogus".into(),
                values: None,
            }]),
            ..Default::default()
        };
        assert!(!selector_matches(&selector, &BTreeMap::new()));
    }
}
