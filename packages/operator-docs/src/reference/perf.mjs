// The resource budget an operator is held to (the kit's
// `Budget::small_cluster()`, with the operator's overrides) and, when the
// operator committed one, what its last `weebo-kit-perf` run measured
// against it — the same checks as the kit's `Report::check`, worst phase
// per limit.

import fs from "node:fs";
import { fileURLToPath } from "node:url";

/** `Budget::small_cluster()`, kept equal by a weebo-kit-perf test. */
export const SMALL_CLUSTER = JSON.parse(
  fs.readFileSync(fileURLToPath(new URL("./small-cluster-budget.json", import.meta.url)), "utf8"),
);

// The kit's idle-window names (`weebo_kit_perf::{BASELINE, FINAL}`).
const BASELINE = "baseline";
const FINAL = "final";

/** Every budget field, in the kit's order: what it limits and its unit. */
export const LIMITS = [
  ["startup_secs", "Startup: spawn → leader lease held", "s"],
  ["shutdown_secs", "Shutdown: SIGTERM → exit", "s"],
  ["idle_rss_mib", "Memory at rest (median RSS of every idle window)", "MiB"],
  ["peak_rss_mib", "Peak memory over the whole run", "MiB"],
  ["idle_cpu_millicores", "CPU at rest (mean of every idle window)", "m"],
  ["cpu_ms_per_object", "CPU time per object of a load", "ms"],
  ["idle_api_requests_per_min", "apiserver requests at rest", "req/min"],
  ["api_requests_per_object", "apiserver requests per object of a load", "req"],
  ["convergence_secs_per_100", "Time for 100 objects of a load to converge", "s"],
  ["reconcile_p95_secs", "Reconcile p95, any phase", "s"],
  ["rss_growth_pct", "Memory not given back: final vs baseline idle window", "%"],
  ["fd_growth", "File descriptors not given back", ""],
  ["thread_growth", "Threads not given back", ""],
];

const max = (values) => (values.length ? Math.max(...values) : undefined);

/** The worst measured value per budget field, as `Report::check` computes them. */
export function measured(report) {
  const idle = report.phases.filter((p) => p.kind === "idle");
  const loads = report.phases.filter((p) => p.kind === "load" && p.objects > 0);
  const base = report.phases.find((p) => p.name === BASELINE);
  const fin = report.phases.find((p) => p.name === FINAL);
  const growth = base && fin;
  return {
    startup_secs: report.startup_secs,
    shutdown_secs: report.shutdown_secs,
    idle_rss_mib: max(idle.map((p) => p.usage.rss_median_mib)),
    peak_rss_mib: max(report.phases.map((p) => p.usage.rss_peak_mib)),
    idle_cpu_millicores: max(idle.map((p) => p.usage.cpu_mean_millicores)),
    cpu_ms_per_object: max(loads.map((p) => (p.usage.cpu_seconds * 1000) / p.objects)),
    idle_api_requests_per_min: max(idle.map((p) => p.api_requests_per_min)),
    api_requests_per_object: max(loads.map((p) => p.api_requests / p.objects)),
    convergence_secs_per_100: max(
      loads.filter((p) => p.convergence_secs != null).map((p) => (p.convergence_secs / p.objects) * 100),
    ),
    reconcile_p95_secs: max(report.phases.filter((p) => p.reconcile_p95_secs != null).map((p) => p.reconcile_p95_secs)),
    rss_growth_pct:
      growth && base.usage.rss_median_mib > 0
        ? ((fin.usage.rss_median_mib - base.usage.rss_median_mib) / base.usage.rss_median_mib) * 100
        : undefined,
    fd_growth: growth ? fin.usage.fds_max - base.usage.fds_max : undefined,
    thread_growth: growth ? fin.usage.threads_max - base.usage.threads_max : undefined,
  };
}

/**
 * What the load phases created: "200 `<resource>`", read from the
 * apiserver requests of the first load (its POSTs), when it can be told.
 */
export function workload(report) {
  const load = report.phases.find((p) => p.kind === "load" && p.objects > 0);
  if (!load) return undefined;
  const post = (load.api_top ?? []).find(([verb, n]) => verb.startsWith("POST ") && n === load.objects && verb !== "POST events");
  return { objects: load.objects, resource: post?.[0].slice("POST ".length) };
}
