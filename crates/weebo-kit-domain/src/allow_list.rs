//! Pure evaluation of `*NamespacePolicy` rules — shared by an operator's
//! admission webhook and its reconcilers' own re-check. Generic over the
//! operator's namespaced kinds (`K`) and over the cluster-scoped CRs a
//! request references, grouped in named dimensions (`"organizations"`,
//! `"users"`…): an operator with nothing to scope by just leaves them empty.

use std::collections::BTreeMap;
use std::fmt::Debug;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule<K> {
    /// Exact names, `*`, or a prefix glob such as `team-*`.
    pub namespaces: Vec<String>,
    pub kinds: Vec<K>,
    /// Per dimension, the CR names the rule is limited to. A dimension
    /// left out = any.
    pub scopes: BTreeMap<String, Vec<String>>,
    pub effect: Effect,
}

/// What a namespaced CR asks for: its namespace, kind, and the
/// cluster-scoped CRs it references, per dimension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request<'a, K> {
    pub namespace: &'a str,
    pub kind: K,
    pub references: BTreeMap<&'a str, &'a [String]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allowed,
    /// Human-readable why, for the webhook message / status condition.
    Denied(String),
}

/// Allow everything while no policy exists at all; default-deny as soon as
/// one does (even with zero rules). `policy_kind` names the policy CRD in
/// denial messages.
///
/// Then a request is allowed only if some `Allow` rule matches its
/// namespace and kind *and* covers every CR it references in each scoped
/// dimension, and no `Deny` rule matches. A `Deny` rule matches on
/// namespace and kind, narrowed by its scopes when set (it then only denies
/// requests referencing one of the listed CRs in every scoped dimension).
/// Deny always wins, whatever the order.
pub fn evaluate<K: PartialEq + Debug>(
    policy_kind: &str,
    request: &Request<'_, K>,
    rules: &[Rule<K>],
    policies_exist: bool,
) -> Decision {
    if !policies_exist {
        return Decision::Allowed;
    }
    let applies = |rule: &Rule<K>| {
        rule.kinds.contains(&request.kind)
            && rule
                .namespaces
                .iter()
                .any(|pattern| namespace_matches(pattern, request.namespace))
    };
    let asked = |dimension: &str| request.references.get(dimension).copied().unwrap_or(&[]);

    for rule in rules
        .iter()
        .filter(|r| r.effect == Effect::Deny && applies(r))
    {
        let hits = rule
            .scopes
            .iter()
            .all(|(dimension, listed)| asked(dimension).iter().any(|a| listed.contains(a)));
        if hits {
            return Decision::Denied(format!(
                "a Deny rule matches namespace {:?} for {:?}",
                request.namespace, request.kind
            ));
        }
    }

    let allowed = rules.iter().any(|r| {
        r.effect == Effect::Allow
            && applies(r)
            && r.scopes
                .iter()
                .all(|(dimension, listed)| asked(dimension).iter().all(|a| listed.contains(a)))
    });
    if allowed {
        return Decision::Allowed;
    }

    let mut why = format!(
        "no {policy_kind} rule allows {:?} in namespace {:?}",
        request.kind, request.namespace
    );
    for (dimension, names) in &request.references {
        if !names.is_empty() {
            why.push_str(&format!(" with {dimension} {names:?}"));
        }
    }
    Decision::Denied(why)
}

fn namespace_matches(pattern: &str, namespace: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => namespace.starts_with(prefix),
        None => pattern == namespace,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Kind {
        Repository,
        AccessToken,
    }

    const POLICY: &str = "TestNamespacePolicy";

    fn rule(ns: &[&str], kinds: &[Kind], effect: Effect) -> Rule<Kind> {
        Rule {
            namespaces: ns.iter().map(|s| s.to_string()).collect(),
            kinds: kinds.to_vec(),
            scopes: BTreeMap::new(),
            effect,
        }
    }

    fn req<'a>(
        ns: &'a str,
        kind: Kind,
        orgs: &'a [String],
        users: &'a [String],
    ) -> Request<'a, Kind> {
        Request {
            namespace: ns,
            kind,
            references: BTreeMap::from([("organizations", orgs), ("users", users)]),
        }
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn eval(request: &Request<'_, Kind>, rules: &[Rule<Kind>]) -> Decision {
        evaluate(POLICY, request, rules, true)
    }

    #[test]
    fn no_policy_allows_everything() {
        assert_eq!(
            evaluate(POLICY, &req("a", Kind::Repository, &[], &[]), &[], false),
            Decision::Allowed
        );
    }

    #[test]
    fn an_existing_policy_switches_to_default_deny() {
        assert!(matches!(
            eval(&req("a", Kind::Repository, &[], &[]), &[]),
            Decision::Denied(_)
        ));
    }

    #[test]
    fn allow_matches_namespace_globs_and_kinds() {
        let rules = [rule(&["team-*"], &[Kind::Repository], Effect::Allow)];
        assert_eq!(
            eval(&req("team-a", Kind::Repository, &[], &[]), &rules),
            Decision::Allowed
        );
        assert!(matches!(
            eval(&req("ops", Kind::Repository, &[], &[]), &rules),
            Decision::Denied(_)
        ));
        assert!(matches!(
            eval(&req("team-a", Kind::AccessToken, &[], &[]), &rules),
            Decision::Denied(_)
        ));
    }

    #[test]
    fn scoped_references_must_all_be_covered() {
        let mut r = rule(
            &["team-a"],
            &[Kind::Repository, Kind::AccessToken],
            Effect::Allow,
        );
        r.scopes.insert("organizations".into(), s(&["weebo"]));
        r.scopes.insert("users".into(), s(&["ci-bot"]));
        let rules = [r];
        let weebo = s(&["weebo"]);
        let other = s(&["other"]);
        let bot = s(&["ci-bot"]);
        let admin = s(&["admin-bot"]);
        assert_eq!(
            eval(&req("team-a", Kind::Repository, &weebo, &[]), &rules),
            Decision::Allowed
        );
        assert!(matches!(
            eval(&req("team-a", Kind::Repository, &other, &[]), &rules),
            Decision::Denied(_)
        ));
        assert_eq!(
            eval(&req("team-a", Kind::AccessToken, &[], &bot), &rules),
            Decision::Allowed
        );
        // The escalation the user list exists for: a token of another user.
        assert!(matches!(
            eval(&req("team-a", Kind::AccessToken, &[], &admin), &rules),
            Decision::Denied(_)
        ));
    }

    #[test]
    fn deny_wins_and_can_be_narrowed() {
        let mut deny = rule(&["*"], &[Kind::AccessToken], Effect::Deny);
        deny.scopes.insert("users".into(), s(&["admin-bot"]));
        let rules = [rule(&["*"], &[Kind::AccessToken], Effect::Allow), deny];
        let bot = s(&["ci-bot"]);
        let admin = s(&["admin-bot"]);
        assert_eq!(
            eval(&req("x", Kind::AccessToken, &[], &bot), &rules),
            Decision::Allowed
        );
        assert!(matches!(
            eval(&req("x", Kind::AccessToken, &[], &admin), &rules),
            Decision::Denied(_)
        ));
    }

    #[test]
    fn denials_name_the_policy_and_the_references() {
        let weebo = s(&["weebo"]);
        let Decision::Denied(why) = eval(&req("ns", Kind::Repository, &weebo, &[]), &[]) else {
            panic!("denied expected");
        };
        assert_eq!(
            why,
            "no TestNamespacePolicy rule allows Repository in namespace \"ns\" \
             with organizations [\"weebo\"]"
        );
    }

    #[test]
    fn unscoped_operators_need_no_references() {
        let rules = [rule(&["apps"], &[Kind::Repository], Effect::Allow)];
        let request = Request {
            namespace: "apps",
            kind: Kind::Repository,
            references: BTreeMap::new(),
        };
        assert_eq!(eval(&request, &rules), Decision::Allowed);
    }
}
