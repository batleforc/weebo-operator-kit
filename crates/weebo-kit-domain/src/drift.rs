//! Write only what differs. A reconcile reads the remote object, compares
//! it with the desired state and writes only when a field the operator
//! owns has drifted: a remote system records every write (audit events,
//! tasks, reloads), and rewriting an unchanged object on every requeue
//! floods it for nothing.
//!
//! [`Drift`] works on the PATCH the operator would send: every field
//! already holding the desired value is cleared (`None`, left out of the
//! request), every field that differs stays and is recorded, for the log
//! or Event naming it. Nothing left: no call. A field the spec doesn't set
//! (`None`) is "no opinion", never compared nor written.
//!
//! Compare to the form the remote system **stores**, not the one sent: a
//! server that trims text, rewrites durations or fills a map with defaults
//! needs the same normalization in the comparison ([`Drift::keep_with`],
//! [`same_trimmed`], [`same_subset`]), or the field drifts on every
//! reconcile. Each such normalization deserves a contract test: the remote
//! value returned normalized, no write expected.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hash};

/// The fields whose desired value differs from the remote one, in the
/// order they were checked.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Drift(Vec<&'static str>);

impl Drift {
    /// Nothing differs: the write can be skipped.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The drifted fields, in the order they were checked.
    pub fn fields(&self) -> &[&'static str] {
        &self.0
    }

    /// `desired` is the PATCH field (`None`: not sent, no opinion); `remote`
    /// the value read back. Equal: `desired` is cleared, the field is left
    /// out of the PATCH. Different: it stays, and `field` is recorded.
    pub fn keep<T: PartialEq>(&mut self, field: &'static str, desired: &mut Option<T>, remote: &T) {
        self.keep_with(field, desired, remote, |d, r| d == r);
    }

    /// [`Drift::keep`] against a remote value the server may leave unset:
    /// an unset remote value never equals a desired one.
    pub fn keep_opt<T: PartialEq>(
        &mut self,
        field: &'static str,
        desired: &mut Option<T>,
        remote: &Option<T>,
    ) {
        self.keep_with(field, desired, remote, |d, r| r.as_ref() == Some(d));
    }

    /// [`Drift::keep`] with the caller's equality — for a list compared as a
    /// set ([`same_set`]), a map compared on its desired keys only
    /// ([`same_subset`]), a text the server normalizes ([`same_trimmed`]),
    /// or a remote value of another type than the request's.
    pub fn keep_with<T, R>(
        &mut self,
        field: &'static str,
        desired: &mut Option<T>,
        remote: &R,
        same: impl FnOnce(&T, &R) -> bool,
    ) {
        if let Some(d) = desired.as_ref() {
            if same(d, remote) {
                *desired = None;
            } else {
                self.0.push(field);
            }
        }
    }

    /// Records `field` when `differs` — for a caller whose request isn't a
    /// PATCH of `Option`s (a full update sent whenever anything drifted),
    /// with the comparison already done.
    pub fn note(&mut self, field: &'static str, differs: bool) {
        if differs {
            self.0.push(field);
        }
    }
}

/// Why a reconcile wrote to the remote system: the `cause` label of the
/// remote-writes counter. At rest, an operator comparing before writing
/// counts none; a `Drift` write coming back on every reconcile for the
/// same kind is a comparison bug (a normalization the server applies and
/// the comparison doesn't), not a drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemoteWrite {
    /// The remote object didn't exist.
    Created,
    /// A field the operator owns no longer held the desired value.
    Drift,
    /// Deleted by the finalizer.
    Deleted,
}

impl RemoteWrite {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Drift => "drift",
            Self::Deleted => "deleted",
        }
    }
}

/// The same elements, whatever their order, duplicates counted — for the
/// lists the remote system treats as sets. `PartialEq` only: these lists
/// hold a handful of elements, a quadratic count is cheaper than requiring
/// `Ord` of every client type.
pub fn same_set<T: PartialEq>(a: &[T], b: &[T]) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|x| a.iter().filter(|y| *y == x).count() == b.iter().filter(|y| *y == x).count())
}

/// Every desired key holds the desired value remotely; other remote keys
/// are ignored — for a map the server completes with its own defaults.
pub fn same_subset<K, V, S1, S2>(desired: &HashMap<K, V, S1>, remote: &HashMap<K, V, S2>) -> bool
where
    K: Hash + Eq,
    V: PartialEq,
    S2: BuildHasher,
{
    desired
        .iter()
        .all(|(k, v)| remote.get(k).is_some_and(|r| r == v))
}

/// Equal once leading and trailing whitespace is ignored — for a text field
/// the server trims before storing it (Django REST Framework's `CharField`
/// does by default): a CR's YAML block scalar (`|`) ends with a `\n` that
/// never comes back, and a byte-for-byte comparison drifts on every
/// reconcile. The PATCH still carries the spec's value untouched; only the
/// comparison is done on the stored form.
pub fn same_trimmed(desired: &str, remote: &str) -> bool {
    desired.trim() == remote.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_equal_field_leaves_the_patch() {
        let mut drift = Drift::default();
        let mut name = Some("docs".to_string());
        drift.keep("name", &mut name, &"docs".to_string());
        assert_eq!(name, None);
        assert!(drift.is_empty());
    }

    #[test]
    fn a_different_field_stays_and_is_recorded() {
        let mut drift = Drift::default();
        let mut name = Some("docs".to_string());
        drift.keep("name", &mut name, &"wiki".to_string());
        assert_eq!(name.as_deref(), Some("docs"));
        assert_eq!(drift.fields(), ["name"]);
    }

    #[test]
    fn a_field_not_sent_is_no_opinion() {
        let mut drift = Drift::default();
        let mut icon: Option<String> = None;
        drift.keep_opt("icon", &mut icon, &Some("x.png".to_string()));
        assert_eq!(icon, None);
        assert!(drift.is_empty());
    }

    #[test]
    fn an_unset_remote_value_differs_from_a_desired_one() {
        let mut drift = Drift::default();
        let mut validity = Some("hours=1".to_string());
        drift.keep_opt("validity", &mut validity, &None);
        assert_eq!(drift.fields(), ["validity"]);
    }

    #[test]
    fn a_nullable_reference_compares_its_inner_value() {
        // `Option<Option<Id>>` fields: `Some(None)` sends an explicit null.
        let mut drift = Drift::default();
        let mut flow: Option<Option<u8>> = Some(None);
        drift.keep_opt("flow", &mut flow, &Some(None));
        assert_eq!(flow, None);
        let mut flow: Option<Option<u8>> = Some(Some(1));
        drift.keep_opt("flow", &mut flow, &Some(None));
        assert_eq!(drift.fields(), ["flow"]);
    }

    #[test]
    fn sets_ignore_order_but_not_duplicates() {
        assert!(same_set(&[1, 2, 3], &[3, 1, 2]));
        assert!(!same_set(&[1, 2], &[1, 2, 3]));
        assert!(!same_set(&[1, 1, 2], &[1, 2, 2]));
        assert!(same_set::<u8>(&[], &[]));
    }

    #[test]
    fn a_subset_ignores_the_keys_the_server_added() {
        let remote = HashMap::from([("log_level", 1), ("host", 2)]);
        assert!(same_subset(&HashMap::from([("log_level", 1)]), &remote));
        assert!(!same_subset(&HashMap::from([("log_level", 3)]), &remote));
        assert!(!same_subset(&HashMap::from([("missing", 1)]), &remote));
    }

    #[test]
    fn trimmed_text_ignores_only_the_outer_whitespace() {
        assert!(same_trimmed("return {}\n", "return {}"));
        assert!(same_trimmed("  return {}\n\n", "return {}"));
        assert!(!same_trimmed("return  {}", "return {}"));
        assert!(!same_trimmed("return {}\nreturn 1\n", "return {}"));
    }

    #[test]
    fn several_fields_are_reported_in_order() {
        let mut drift = Drift::default();
        let (mut a, mut b, mut c) = (Some(1), Some(2), Some(3));
        drift.keep("a", &mut a, &9);
        drift.keep("b", &mut b, &2);
        drift.note("n", false);
        drift.keep("c", &mut c, &9);
        drift.note("d", true);
        assert_eq!(drift.fields(), ["a", "c", "d"]);
        assert_eq!((a, b, c), (Some(1), None, Some(3)));
    }
}
