fn metrics() {
    ReconcileMetrics::new("example-operator", "weebo_example");
    meter.u64_observable_gauge("weebo_example_leader");
    meter.u64_counter("weebo_example_remote_calls");
}

fn wrote(metrics: &ReconcileMetrics) {
    metrics.record_write("ExampleWidget", RemoteWrite::Drift);
}

fn admit() -> Result<(), Denial<ReasonCode>> {
    Err(Denial::new(
        ReasonCode::NotAllowed,
        "refused",
    ))
}

fn reason(err: &Error) -> (ReasonCode, &'static str) {
    match err {
        Error::Kube(_) => (ReasonCode::KubernetesApiError, events::RECONCILE),
        Error::Remote(_) => (ReasonCode::Unreachable, events::RECONCILE),
    }
}

fn reconcile() -> Failure {
    Failure::new(ReasonCode::Unreachable, "down")
}

async fn created() {
    events::announce_change(
        recorder,
        &object,
        ReasonCode::Created,
        events::RECONCILE,
        "created",
    )
    .await;
    events::announce_drift(recorder, &object, ReasonCode::DriftCorrected, &drift).await;
    weebo_kit_runtime::controller::finalize(obj, recorder, annotations, ReasonCode::Released, cleanup).await;
}
