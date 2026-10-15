//! The operator's own pod, as the chart describes it: its selector labels
//! (`WEEBO_OPERATOR_POD_LABELS`) and namespace (`POD_NAMESPACE`). What a
//! NetworkPolicy the operator writes selects to let the operator itself
//! reach what it deploys.

use std::collections::BTreeMap;

/// The operator pod's selector labels, a JSON object of strings (the
/// chart's `weebo-operator.selectorLabels`, `nameOverride` applied).
pub const POD_LABELS_ENV: &str = "WEEBO_OPERATOR_POD_LABELS";

/// The operator pod's namespace (downward API).
pub const POD_NAMESPACE_ENV: &str = "POD_NAMESPACE";

/// [`POD_LABELS_ENV`]: `Ok(None)` when unset (run outside the chart),
/// an error when it isn't a JSON object of strings — a broken chart, to
/// stop the start rather than write a policy selecting nothing.
pub fn own_labels() -> Result<Option<BTreeMap<String, String>>, String> {
    match std::env::var(POD_LABELS_ENV) {
        Ok(raw) => parse_labels(&raw).map(Some),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(e) => Err(format!("{POD_LABELS_ENV}: {e}")),
    }
}

/// [`POD_NAMESPACE_ENV`], when set.
pub fn own_namespace() -> Option<String> {
    std::env::var(POD_NAMESPACE_ENV)
        .ok()
        .filter(|ns| !ns.is_empty())
}

fn parse_labels(raw: &str) -> Result<BTreeMap<String, String>, String> {
    let labels: BTreeMap<String, String> = serde_json::from_str(raw)
        .map_err(|e| format!("{POD_LABELS_ENV} is not a JSON object of strings: {e}"))?;
    if labels.is_empty() {
        return Err(format!(
            "{POD_LABELS_ENV} is empty: it would select every pod"
        ));
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
        assert!(parse_labels("{}").unwrap_err().contains("every pod"));
        assert!(parse_labels("app=x").is_err());
        assert!(parse_labels(r#"{"a": 1}"#).is_err());
    }
}
