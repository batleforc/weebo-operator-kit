//! A `*Ref` field — a CR pointing at another CR of the same operator — and
//! whether it can be used yet. The referenced CR must exist, target the
//! same instance, and be `Ready` with the remote object it stands for;
//! what that object's identifier is (`i64` + name, a UUID…) is `Id`.
//!
//! Reading the referenced CR is the operator's (it knows its kinds and
//! their status): it hands over a [`Referenced`]. Errors stay neutral
//! ([`RefError`]): the operator maps them onto its reason codes.

use weebo_kit_api::BoundInstance;

/// What a ref needs to know of the referenced CR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Referenced<Id> {
    /// The instance its status records (`None` before its first
    /// resolution).
    pub instance: Option<BoundInstance>,
    /// The remote object recorded in its status, if any.
    pub remote: Option<Id>,
    /// Its `Ready` condition: status, reason.
    pub ready: Option<(bool, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefState<Id> {
    Ready(Id),
    NotFound,
    /// Exists but has no `Ready` condition yet (never reconciled): the
    /// operator names it with its own reason code.
    NotReconciled,
    /// Exists but isn't usable yet; carries the referenced CR's `Ready`
    /// reason.
    NotReady(String),
    /// Bound to another instance than the referencing CR: an identifier
    /// from one instance means nothing on another.
    OtherInstance(BoundInstance),
}

/// Why a required ref can't be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefError {
    NotFound,
    NotReconciled,
    NotReady(String),
    OtherInstance(BoundInstance),
}

impl<Id> RefState<Id> {
    /// The state of a ref to `found` (`None`: no such CR) from a CR bound
    /// to `ours`: the same instance (kind, namespace and name — see
    /// [`same_instance`]) or nothing.
    pub fn of(found: Option<Referenced<Id>>, ours: &BoundInstance) -> Self {
        let Some(cr) = found else {
            return RefState::NotFound;
        };
        if let Some(theirs) = cr.instance
            && !same_instance(&theirs, ours)
        {
            return RefState::OtherInstance(theirs);
        }
        match (cr.remote, cr.ready) {
            (Some(id), Some((true, _))) => RefState::Ready(id),
            (_, Some((_, reason))) => RefState::NotReady(reason),
            (_, None) => RefState::NotReconciled,
        }
    }

    /// A required ref: anything but `Ready` is an error.
    pub fn require(self) -> Result<Id, RefError> {
        match self {
            RefState::Ready(id) => Ok(id),
            RefState::NotFound => Err(RefError::NotFound),
            RefState::NotReconciled => Err(RefError::NotReconciled),
            RefState::NotReady(why) => Err(RefError::NotReady(why)),
            RefState::OtherInstance(theirs) => Err(RefError::OtherInstance(theirs)),
        }
    }
}

impl RefError {
    /// `kind`/`name`: the referenced CR.
    pub fn describe(&self, kind: &str, name: &str) -> String {
        match self {
            RefError::NotFound => format!("{kind} {name:?} does not exist"),
            RefError::NotReconciled => format!("{kind} {name:?} has not been reconciled yet"),
            RefError::NotReady(why) => format!("{kind} {name:?} is not ready yet ({why})"),
            RefError::OtherInstance(theirs) => {
                format!("{kind} {name:?} is bound to {theirs}, not to this CR's instance")
            }
        }
    }
}

/// Two CRs live on the same instance: their resolved instances (as their
/// statuses record them) are the same kind, namespace and name. A
/// cluster-scoped instance and a local one of the same name are not.
pub fn same_instance(theirs: &BoundInstance, ours: &BoundInstance) -> bool {
    theirs == ours
}

#[cfg(test)]
mod tests {
    use super::*;

    fn main() -> BoundInstance {
        BoundInstance::cluster("main")
    }

    fn cr(remote: Option<u8>, ready: Option<(bool, &str)>) -> Option<Referenced<u8>> {
        Some(Referenced {
            instance: Some(main()),
            remote,
            ready: ready.map(|(b, r)| (b, r.to_string())),
        })
    }

    #[test]
    fn a_ref_is_ready_with_a_remote_object_and_ready_true() {
        assert_eq!(
            RefState::of(cr(Some(7), Some((true, "Reconciled"))), &main()),
            RefState::Ready(7)
        );
        assert_eq!(
            RefState::of(cr(Some(7), Some((false, "Unreachable"))), &main()),
            RefState::NotReady("Unreachable".into())
        );
        assert_eq!(
            RefState::of(cr(None, None), &main()),
            RefState::NotReconciled
        );
        assert_eq!(RefState::<u8>::of(None, &main()), RefState::NotFound);
        let local = BoundInstance::local("team-a", "main");
        assert_eq!(
            RefState::of(cr(Some(7), Some((true, "Reconciled"))), &local),
            RefState::OtherInstance(main())
        );
    }

    #[test]
    fn a_cr_not_bound_yet_is_not_ready_rather_than_elsewhere() {
        let unbound = Some(Referenced::<u8> {
            instance: None,
            remote: None,
            ready: None,
        });
        assert_eq!(RefState::of(unbound, &main()), RefState::NotReconciled);
    }

    #[test]
    fn errors_describe_the_ref() {
        let e = RefState::of(
            cr(Some(7), Some((true, "R"))),
            &BoundInstance::local("team-a", "main"),
        )
        .require()
        .unwrap_err();
        assert_eq!(
            e.describe("XUser", "alice"),
            "XUser \"alice\" is bound to Instance main, not to this CR's instance"
        );
        assert_eq!(
            RefError::NotFound.describe("XUser", "bob"),
            "XUser \"bob\" does not exist"
        );
        assert_eq!(
            RefState::of(cr(None, None), &main()).require(),
            Err::<u8, _>(RefError::NotReconciled)
        );
        assert_eq!(
            RefError::NotReconciled.describe("XUser", "carol"),
            "XUser \"carol\" has not been reconciled yet"
        );
    }
}
