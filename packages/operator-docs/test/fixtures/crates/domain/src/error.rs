//! The fixture operator's catalog.

use weebo_kit_domain::{Reason, Severity, reason_codes};

reason_codes! {
    /// Every reason.
    pub enum ReasonCode {
        /// `Ready: True`.
        Reconciled,
        /// The remote object is unreachable.
        ///
        /// Retried. See `application::ports::GatewayError`.
        Unreachable,
        #[allow(dead_code)]
        /// Refused by the allow-list.
        NotAllowed,
        /// A member is missing (see `.prompt/plan.md`, "Scope"); the rest syncs.
        MemberPending,
        /// The remote object was created (Event only).
        Created,
        /// The finalizer finished (Event only).
        Released,
        /// A Kubernetes API call failed before any status.
        KubernetesApiError
    }
}

impl ReasonCode {
    pub const fn severity(self) -> Severity {
        match self {
            ReasonCode::Reconciled
            | ReasonCode::MemberPending
            | ReasonCode::Created
            | ReasonCode::Released => Severity::Advisory,
            _ => Severity::Blocking,
        }
    }
}

impl Reason for ReasonCode {
    const SUCCESS: Self = ReasonCode::Reconciled;
    fn as_str(self) -> &'static str {
        ReasonCode::as_str(self)
    }
    fn severity(self) -> Severity {
        ReasonCode::severity(self)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn x() {
        let _ = super::ReasonCode::Unreachable;
    }
}
