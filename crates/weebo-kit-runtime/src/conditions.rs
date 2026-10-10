//! A reconcile's result → the condition list of a status, and the Event
//! announcing a `Ready` change. Only reason codes ever reach a condition
//! or an Event.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use kube::runtime::events::{Event, EventType, Recorder};
use weebo_kit_api::Condition;
use weebo_kit_api::status::ready;
use weebo_kit_domain::{Advisory, Failure, Reason};

/// The full condition list: `Ready` first, then one condition per advisory
/// (its `type` is the reason's name). `lastTransitionTime` only moves when
/// a condition's status flips.
pub fn conditions<R: Reason>(
    previous: &[Condition],
    outcome: &Result<String, Failure<R>>,
    advisories: &[Advisory<R>],
    generation: Option<i64>,
    now: &Time,
) -> Vec<Condition> {
    let condition = |type_: &str, status: &str, reason: R, message: &str| {
        let prev = previous.iter().find(|c| c.type_ == type_);
        let last_transition_time = match prev {
            Some(p) if p.status == status => p.last_transition_time.clone(),
            _ => Some(now.clone()),
        };
        Condition {
            type_: type_.to_string(),
            status: status.to_string(),
            reason: reason.as_str().to_string(),
            message: message.to_string(),
            last_transition_time,
            observed_generation: generation,
        }
    };

    let mut out = vec![match outcome {
        Ok(message) => condition(Condition::READY, "True", R::SUCCESS, message),
        Err(f) => condition(Condition::READY, "False", f.reason, &f.message),
    }];
    for advisory in advisories {
        out.push(condition(
            advisory.reason.as_str(),
            "True",
            advisory.reason,
            &advisory.message,
        ));
    }
    out
}

/// Emits an Event when `Ready`'s status or reason changed between the
/// `previous` and `current` condition lists — transitions only; prefer
/// [`crate::events::announce_outcome`], which also warns on every repeated
/// failure. An Event is a courtesy:
/// failing to record one is logged, never returned (it must not fail a
/// reconcile whose status patch already succeeded).
pub async fn announce_ready_change(
    recorder: &Recorder,
    object: &k8s_openapi::api::core::v1::ObjectReference,
    previous: &[Condition],
    current: &[Condition],
) {
    let Some(now) = ready(current) else {
        return;
    };
    let before = ready(previous).map(|c| (c.status.as_str(), c.reason.as_str()));
    if before == Some((now.status.as_str(), now.reason.as_str())) {
        return;
    }
    let event = Event {
        type_: if now.status == "True" {
            EventType::Normal
        } else {
            EventType::Warning
        },
        reason: now.reason.clone(),
        // Events cap the note at 1 KiB.
        note: Some(now.message.chars().take(1000).collect()),
        action: "Reconcile".to_string(),
        secondary: None,
    };
    if let Err(err) = recorder.publish(&event, object).await {
        tracing::warn!(error = %err, "failed to publish event");
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::jiff::Timestamp;
    use weebo_kit_domain::Severity;

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Code {
        Reconciled,
        Pending,
        Unreachable,
    }

    impl Reason for Code {
        const SUCCESS: Self = Code::Reconciled;
        fn as_str(self) -> &'static str {
            match self {
                Code::Reconciled => "Reconciled",
                Code::Pending => "Pending",
                Code::Unreachable => "Unreachable",
            }
        }
        fn severity(self) -> Severity {
            match self {
                Code::Unreachable => Severity::Blocking,
                _ => Severity::Advisory,
            }
        }
    }

    fn at(secs: i64) -> Time {
        Time(Timestamp::from_second(secs).unwrap())
    }

    #[test]
    fn transition_time_only_moves_when_status_flips() {
        let ok: Result<String, Failure<Code>> = Ok("fine".into());
        let first = conditions(&[], &ok, &[], Some(1), &at(10));
        assert_eq!(first[0].last_transition_time, Some(at(10)));
        assert_eq!(first[0].reason, "Reconciled");

        let again = conditions(&first, &ok, &[], Some(2), &at(20));
        assert_eq!(again[0].last_transition_time, Some(at(10)));
        assert_eq!(again[0].observed_generation, Some(2));

        let failed = Err(Failure::new(Code::Unreachable, "x"));
        let flipped = conditions(&again, &failed, &[], Some(2), &at(30));
        assert_eq!(flipped[0].last_transition_time, Some(at(30)));
        assert_eq!(flipped[0].reason, "Unreachable");
    }

    #[test]
    fn advisories_become_their_own_conditions() {
        let advisories = [Advisory {
            reason: Code::Pending,
            message: "carol".into(),
        }];
        let conds = conditions(&[], &Ok("fine".into()), &advisories, None, &at(1));
        assert_eq!(conds.len(), 2);
        assert_eq!(conds[1].type_, "Pending");
        assert_eq!(conds[1].status, "True");
    }
}
