//! The operator's own pod, as the chart describes it: its selector labels
//! (`WEEBO_OPERATOR_POD_LABELS`) and namespace (`POD_NAMESPACE`). What a
//! NetworkPolicy the operator writes selects to let the operator itself
//! reach what it deploys.

use std::collections::BTreeMap;

use weebo_kit_application::workload::OperatorPod;

/// The operator pod's selector labels, a JSON object of strings (the
/// chart's `weebo-operator.selectorLabels`, `nameOverride` applied).
pub const POD_LABELS_ENV: &str = "WEEBO_OPERATOR_POD_LABELS";

/// The operator pod's namespace (downward API).
pub const POD_NAMESPACE_ENV: &str = "POD_NAMESPACE";

/// A [`POD_LABELS_ENV`] the operator can't select its own pods with.
#[derive(Debug, thiserror::Error)]
pub enum PodLabelsError {
    #[error("{POD_LABELS_ENV}: {0}")]
    Env(std::env::VarError),
    #[error("{POD_LABELS_ENV} is not a JSON object of strings: {0}")]
    NotJson(serde_json::Error),
    #[error("{POD_LABELS_ENV} is empty: it would select every pod")]
    Empty,
}

/// [`POD_LABELS_ENV`]: `Ok(None)` when unset (run outside the chart),
/// an error when it isn't a JSON object of strings — a broken chart, to
/// stop the start rather than write a policy selecting nothing.
pub fn own_labels() -> Result<Option<BTreeMap<String, String>>, PodLabelsError> {
    match std::env::var(POD_LABELS_ENV) {
        Ok(raw) => parse_labels(&raw).map(Some),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(e) => Err(PodLabelsError::Env(e)),
    }
}

/// [`POD_NAMESPACE_ENV`], when set.
pub fn own_namespace() -> Option<String> {
    std::env::var(POD_NAMESPACE_ENV)
        .ok()
        .filter(|ns| !ns.is_empty())
}

/// The operator's own pods ([`own_labels`], [`own_namespace`]). Unset (run
/// outside the chart), the chart's default selector label for `chart` —
/// said once, a `nameOverride` would leave the operator out.
pub fn operator_pod(chart: &str) -> Result<OperatorPod, PodLabelsError> {
    let mut operator = OperatorPod::named(chart);
    match own_labels()? {
        Some(labels) => operator.labels = labels,
        None => tracing::warn!(
            fallback = ?operator.labels,
            "{POD_LABELS_ENV} is unset: a NetworkPolicy the operator writes lets in the \
             operator pods by this label, which a chart nameOverride would not match"
        ),
    }
    operator.namespace = own_namespace();
    Ok(operator)
}

fn parse_labels(raw: &str) -> Result<BTreeMap<String, String>, PodLabelsError> {
    let labels: BTreeMap<String, String> =
        serde_json::from_str(raw).map_err(PodLabelsError::NotJson)?;
    if labels.is_empty() {
        return Err(PodLabelsError::Empty);
    }
    Ok(labels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_charts_labels_parse() {
        let labels = parse_labels(
            r#"{"app.kubernetes.io/instance":"ops","app.kubernetes.io/name":"weebo-example"}"#,
        )
        .unwrap();
        assert_eq!(labels["app.kubernetes.io/name"], "weebo-example");
        assert_eq!(labels.len(), 2);
        assert!(
            parse_labels("{}")
                .unwrap_err()
                .to_string()
                .contains("every pod")
        );
        assert!(parse_labels("app=x").is_err());
        assert!(parse_labels(r#"{"a": 1}"#).is_err());
    }
}
