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
