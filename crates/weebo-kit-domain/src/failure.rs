//! What a reconcile ends on besides success, generic over the operator's
//! reason catalog. Operators alias them (`type Failure =
//! weebo_kit_domain::Failure<ReasonCode>`).

/// A `Ready: False` cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure<R> {
    pub reason: R,
    pub message: String,
}

impl<R> Failure<R> {
    pub fn new(reason: R, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }
}

impl<R: std::fmt::Display> std::fmt::Display for Failure<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.reason, self.message)
    }
}

/// A non-blocking condition shown next to `Ready: True` (its `type` is the
/// reason's name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advisory<R> {
    pub reason: R,
    pub message: String,
}

/// A condition of the operator's own `type` (`UpstreamTrusted`), beside
/// `Ready` and the advisories: one facet of the object, which can be
/// `True`, `False` or `Unknown` while `Ready` says whether it works. The
/// reason's severity says nothing here: an aspect never blocks `Ready` by
/// itself, the reconcile decides that through its outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aspect<R> {
    /// PascalCase, never `Ready` nor an advisory reason's name.
    pub type_: &'static str,
    pub status: AspectStatus,
    pub reason: R,
    pub message: String,
}

/// The `status` of an [`Aspect`] condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AspectStatus {
    True,
    False,
    Unknown,
}

impl AspectStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            AspectStatus::True => "True",
            AspectStatus::False => "False",
            AspectStatus::Unknown => "Unknown",
        }
    }
}
