//! The `pathGrants` CRD schema (OpenAPI `pattern` plus CEL `matches`
//! rules) accepts a path pattern if and only if the domain's `validate`
//! does: the apiserver and the stores never disagree.

use regex::Regex;
use weebo_kit_api::VaultPathGrant;
use weebo_kit_domain::vault_paths::{self, PathGrant};

/// The regex of a CEL rule `[!]self.matches('<regex>')`.
fn cel_regex(rule: &str) -> (bool, Regex) {
    let negated = rule.starts_with('!');
    let inner = rule
        .trim_start_matches('!')
        .strip_prefix("self.matches('")
        .and_then(|r| r.strip_suffix("')"))
        .unwrap_or_else(|| panic!("not a matches() rule: {rule}"));
    (negated, Regex::new(&inner.replace(r"\\", r"\")).unwrap())
}

#[test]
fn schema_and_domain_accept_the_same_path_patterns() {
    let schema = serde_json::to_value(schemars::schema_for!(VaultPathGrant)).unwrap();
    let item = &schema["properties"]["paths"]["items"];
    let pattern = Regex::new(item["pattern"].as_str().unwrap()).unwrap();
    let rules: Vec<(bool, Regex)> = item["x-kubernetes-validations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| cel_regex(r["rule"].as_str().unwrap()))
        .collect();
    let schema_accepts = |p: &str| {
        pattern.is_match(p) && rules.iter().all(|(negated, re)| re.is_match(p) != *negated)
    };
    let namespaces = vec!["auth".to_string()];
    for candidate in [
        "shared/auth",
        "a",
        "+/auth",
        "a/+/c",
        "a/+/c/*",
        "shared/*",
        "+/auth/*",
        "a.b/c-d_e",
        "...",
        "*",
        "+",
        "+/*",
        "+/+",
        "a/*/b",
        "/a",
        "a/",
        "a//b",
        "",
        "a/../b",
        "..",
        "a/.",
        "./a",
        "a*",
        "b+/c",
        "a/b*",
    ] {
        let paths = vec![candidate.to_string()];
        let grant = PathGrant {
            namespaces: &namespaces,
            cluster_scoped: false,
            paths: &paths,
        };
        assert_eq!(
            schema_accepts(candidate),
            vault_paths::validate(&[grant]).is_ok(),
            "{candidate:?}"
        );
    }
}
