//! Kubernetes Events for every error an operator hits on a CR and every
//! change of its state — so `kubectl describe` and `kubectl get events`
//! tell a CR's story without reading the operator's logs.
//!
//! - [`announce_outcome`]: after each status patch. A `Warning` every time
//!   the reconcile ends on `Ready: False`; a `Normal` when `Ready` turns
//!   (or stays, with a new reason) `True`, and for every other condition
//!   that appears, changes or goes away.
//! - [`announce_change`]: a lifecycle step the conditions don't show (the
//!   remote object created, deleted by the finalizer…), under one of the
//!   operator's reason codes.
//! - [`announce_error`] / [`spawn_error`]: a reconcile that failed before
//!   reaching a status (an API call, a finalizer cleanup), typically from
//!   the controller's `error_policy`.
//!
//! Repeating the same Event is cheap: kube's `Recorder` folds identical
//! Events (same reason, note, action) into a series and only bumps its
//! count, so a CR failing on every requeue doesn't flood the apiserver —
//! as long as the operator shares one `Recorder`.
//!
//! An Event is a courtesy: failing to publish one is logged, never
//! returned (it must not fail a reconcile).

use k8s_openapi::api::core::v1::ObjectReference;
use kube::runtime::events::{Event, EventType, Recorder};
use weebo_kit_api::Condition;
use weebo_kit_api::status::ready;
use weebo_kit_domain::Reason;

/// `action` of the Events published after a reconcile.
pub const RECONCILE: &str = "Reconcile";
/// `action` of the Events of a finalizer cleanup.
pub const CLEANUP: &str = "Cleanup";

/// Events cap the note at 1 KiB.
const MAX_NOTE_CHARS: usize = 1000;

/// The Events a reconcile that went from `previous` to `current`
/// deserves, `Ready`'s first.
pub fn outcome_events(previous: &[Condition], current: &[Condition]) -> Vec<Event> {
    let mut out = Vec::new();
    if let Some(now) = ready(current) {
        if now.status != "True" {
            out.push(event(
                EventType::Warning,
                &now.reason,
                RECONCILE,
                &now.message,
            ));
        } else if !same_state(ready(previous), now) {
            out.push(event(
                EventType::Normal,
                &now.reason,
                RECONCILE,
                &now.message,
            ));
        }
    }
    for cond in current.iter().filter(|c| c.type_ != Condition::READY) {
        if !same_state(find(previous, &cond.type_), cond) {
            out.push(event(
                EventType::Normal,
                &cond.reason,
                RECONCILE,
                &cond.message,
            ));
        }
    }
    for gone in previous.iter().filter(|c| c.type_ != Condition::READY) {
        if find(current, &gone.type_).is_none() {
            out.push(event(
                EventType::Normal,
                &gone.reason,
                RECONCILE,
                &format!("{} no longer applies: {}", gone.type_, gone.message),
            ));
        }
    }
    out
}

fn find<'a>(list: &'a [Condition], type_: &str) -> Option<&'a Condition> {
    list.iter().find(|c| c.type_ == type_)
}

fn same_state(before: Option<&Condition>, now: &Condition) -> bool {
    before.is_some_and(|b| b.status == now.status && b.reason == now.reason)
}

/// Publishes [`outcome_events`] for `object`.
pub async fn announce_outcome(
    recorder: &Recorder,
    object: &ObjectReference,
    previous: &[Condition],
    current: &[Condition],
) {
    for event in outcome_events(previous, current) {
        publish(recorder, object, &event).await;
    }
}

/// A `Normal` Event for a state change the conditions don't carry (the
/// remote object created, deleted…). `action` is [`RECONCILE`] or
/// [`CLEANUP`].
pub async fn announce_change<R: Reason>(
    recorder: &Recorder,
    object: &ObjectReference,
    reason: R,
    action: &str,
    message: &str,
) {
    publish(
        recorder,
        object,
        &event(EventType::Normal, reason.as_str(), action, message),
    )
    .await;
}

/// A `Warning` Event for an error that ended a reconcile (or a cleanup:
/// `action` is [`RECONCILE`] or [`CLEANUP`]) before any status was
/// written. `reason` is the operator's code for it.
pub async fn announce_error<R: Reason>(
    recorder: &Recorder,
    object: &ObjectReference,
    reason: R,
    action: &str,
    message: &str,
) {
    publish(
        recorder,
        object,
        &event(EventType::Warning, reason.as_str(), action, message),
    )
    .await;
}

/// [`announce_error`] in the background, for `error_policy`, which can't
/// await. Needs a Tokio runtime (the controller's).
pub fn spawn_error<R: Reason>(
    recorder: &Recorder,
    object: ObjectReference,
    reason: R,
    action: &'static str,
    message: String,
) {
    let recorder = recorder.clone();
    tokio::spawn(async move {
        announce_error(&recorder, &object, reason, action, &message).await;
    });
}

fn event(type_: EventType, reason: &str, action: &str, message: &str) -> Event {
    Event {
        type_,
        reason: reason.to_string(),
        note: Some(message.chars().take(MAX_NOTE_CHARS).collect()),
        action: action.to_string(),
        secondary: None,
    }
}

async fn publish(recorder: &Recorder, object: &ObjectReference, event: &Event) {
    if let Err(err) = recorder.publish(event, object).await {
        tracing::warn!(error = %err, reason = %event.reason, "failed to publish event");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_cond(status: &str, reason: &str) -> Vec<Condition> {
        vec![Condition {
            type_: Condition::READY.to_string(),
            status: status.to_string(),
            reason: reason.to_string(),
            message: format!("{reason} happened"),
            ..Default::default()
        }]
    }

    fn advisory(reason: &str) -> Condition {
        Condition {
            type_: reason.to_string(),
            status: "True".to_string(),
            reason: reason.to_string(),
            message: "carol".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn every_failed_reconcile_warns() {
        let failing = ready_cond("False", "Unreachable");
        for previous in [vec![], ready_cond("True", "Reconciled"), failing.clone()] {
            let events = outcome_events(&previous, &failing);
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].type_, EventType::Warning);
            assert_eq!(events[0].reason, "Unreachable");
            assert_eq!(events[0].note.as_deref(), Some("Unreachable happened"));
            assert_eq!(events[0].action, RECONCILE);
        }
    }

    #[test]
    fn success_is_only_announced_when_it_changes() {
        let ok = ready_cond("True", "Reconciled");
        assert_eq!(outcome_events(&[], &ok)[0].type_, EventType::Normal);
        assert_eq!(
            outcome_events(&ready_cond("False", "Unreachable"), &ok)[0].type_,
            EventType::Normal
        );
        assert!(outcome_events(&ok, &ok).is_empty());
        assert!(outcome_events(&ok, &[]).is_empty());
    }

    #[test]
    fn advisories_are_announced_when_they_appear_change_or_go() {
        let ok = ready_cond("True", "Reconciled");
        let mut with = ok.clone();
        with.push(advisory("MemberPending"));

        let appeared = outcome_events(&ok, &with);
        assert_eq!(appeared.len(), 1);
        assert_eq!(appeared[0].reason, "MemberPending");
        assert_eq!(appeared[0].type_, EventType::Normal);

        assert!(outcome_events(&with, &with).is_empty());

        let gone = outcome_events(&with, &ok);
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].reason, "MemberPending");
        assert!(
            gone[0]
                .note
                .as_deref()
                .unwrap()
                .contains("no longer applies")
        );
    }

    #[test]
    fn notes_are_capped() {
        let e = event(EventType::Warning, "R", RECONCILE, &"x".repeat(5000));
        assert_eq!(e.note.unwrap().len(), MAX_NOTE_CHARS);
    }
}
