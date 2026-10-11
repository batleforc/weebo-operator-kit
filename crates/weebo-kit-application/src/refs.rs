//! A `*Ref` field — a CR pointing at another CR of the same operator — and
//! whether it can be used yet. The referenced CR must exist, target the
//! same instance, and be `Ready` with the remote object it stands for;
//! what that object's identifier is (`i64` + name, a UUID…) is `Id`.
//!
//! Reading the referenced CR is the operator's (it knows its kinds and
//! their status): it hands over a [`Referenced`]. Errors stay neutral
//! ([`RefError`]): the operator maps them onto its reason codes.

use std::future::Future;

/// What a ref needs to know of the referenced CR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Referenced<Id> {
    /// Its `spec.instanceRef`.
    pub instance_ref: Option<String>,
    /// The remote object recorded in its status, if any.
    pub remote: Option<Id>,
    /// Its `Ready` condition: status, reason.
    pub ready: Option<(bool, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefState<Id> {
    Ready(Id),
    NotFound,
    /// Exists but isn't usable yet; carries the referenced CR's `Ready`
    /// reason (`NotReconciledYet` before its first reconcile).
    NotReady(String),
    /// Targets another instance than the referencing CR: an identifier
    /// from one instance means nothing on another.
    OtherInstance(Option<String>),
}

/// Why a required ref can't be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefError {
    NotFound,
    NotReady(String),
    OtherInstance(Option<String>),
}

impl<Id> RefState<Id> {
    /// The state of a ref to `found` (`None`: no such CR), knowing whether
    /// it targets the referencing CR's instance.
    pub fn of(found: Option<Referenced<Id>>, same_instance: bool) -> Self {
        let Some(cr) = found else {
            return RefState::NotFound;
        };
        if !same_instance {
            return RefState::OtherInstance(cr.instance_ref);
        }
        match (cr.remote, cr.ready) {
            (Some(id), Some((true, _))) => RefState::Ready(id),
            (_, Some((_, reason))) => RefState::NotReady(reason),
            (_, None) => RefState::NotReady("NotReconciledYet".to_string()),
        }
    }

    /// A required ref: anything but `Ready` is an error.
    pub fn require(self) -> Result<Id, RefError> {
        match self {
            RefState::Ready(id) => Ok(id),
            RefState::NotFound => Err(RefError::NotFound),
            RefState::NotReady(why) => Err(RefError::NotReady(why)),
            RefState::OtherInstance(theirs) => Err(RefError::OtherInstance(theirs)),
        }
    }
}

impl RefError {
    /// `kind`/`name`: the referenced CR; `instance_kind`: the operator's
    /// instance CRD, named when the ref targets another one.
    pub fn describe(&self, kind: &str, name: &str, instance_kind: &str) -> String {
        match self {
            RefError::NotFound => format!("{kind} {name:?} does not exist"),
            RefError::NotReady(why) => format!("{kind} {name:?} is not ready yet ({why})"),
            RefError::OtherInstance(theirs) => format!(
                "{kind} {name:?} targets {instance_kind} {:?}, not this CR's",
                theirs.as_deref().unwrap_or("<default>")
            ),
        }
    }
}

/// Two `instanceRef`s designate the same instance: equal, or one empty
/// while the other names the cluster's only instance — `default_instance`
/// is only awaited in that case.
pub async fn same_instance<E, F>(
    theirs: Option<&str>,
    ours: Option<&str>,
    default_instance: impl FnOnce() -> F,
) -> Result<bool, E>
where
    F: Future<Output = Result<Option<String>, E>>,
{
    Ok(match (theirs, ours) {
        (a, b) if a == b => true,
        (Some(named), None) | (None, Some(named)) => {
            default_instance().await?.as_deref() == Some(named)
        }
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cr(remote: Option<u8>, ready: Option<(bool, &str)>) -> Option<Referenced<u8>> {
        Some(Referenced {
            instance_ref: Some("main".into()),
            remote,
            ready: ready.map(|(b, r)| (b, r.to_string())),
        })
    }

    #[test]
    fn a_ref_is_ready_with_a_remote_object_and_ready_true() {
        assert_eq!(
            RefState::of(cr(Some(7), Some((true, "Reconciled"))), true),
            RefState::Ready(7)
        );
        assert_eq!(
            RefState::of(cr(Some(7), Some((false, "Unreachable"))), true),
            RefState::NotReady("Unreachable".into())
        );
        assert_eq!(
            RefState::of(cr(None, None), true),
            RefState::NotReady("NotReconciledYet".into())
        );
        assert_eq!(RefState::<u8>::of(None, true), RefState::NotFound);
        assert_eq!(
            RefState::of(cr(Some(7), Some((true, "Reconciled"))), false),
            RefState::OtherInstance(Some("main".into()))
        );
    }

    #[test]
    fn errors_describe_the_ref() {
        let e = RefState::of(cr(Some(7), Some((true, "R"))), false)
            .require()
            .unwrap_err();
        assert_eq!(
            e.describe("XUser", "alice", "XInstance"),
            "XUser \"alice\" targets XInstance \"main\", not this CR's"
        );
        assert_eq!(
            RefError::NotFound.describe("XUser", "bob", "XInstance"),
            "XUser \"bob\" does not exist"
        );
    }

    #[tokio::test]
    async fn an_empty_instance_ref_is_the_only_instance() {
        let only = || async { Ok::<_, ()>(Some("main".to_string())) };
        assert!(same_instance(Some("a"), Some("a"), only).await.unwrap());
        assert!(same_instance(None, Some("main"), only).await.unwrap());
        assert!(!same_instance(None, Some("other"), only).await.unwrap());
        assert!(!same_instance(Some("a"), Some("b"), only).await.unwrap());
        let unused = || async { Err::<Option<String>, _>("must not be read") };
        assert!(same_instance(None, None, unused).await.unwrap());
    }
}
