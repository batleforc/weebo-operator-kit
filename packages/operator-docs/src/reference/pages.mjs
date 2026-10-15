// The four reference pages every operator's site carries, rendered as
// Markdown (tables included) so the search index and llms.txt read them
// like any other page. The text is the kit's behaviour, written once; only
// the data (codes, metric prefix, annotation domain, measurements) is the
// operator's. Nothing here names a product.

import * as yaml from "js-yaml";
import { LIMITS, measured, workload } from "./perf.mjs";

const cell = (text) => String(text ?? "").replace(/\|/g, "\\|").replace(/\r?\n+/g, " ");
const code = (c) => `\`${c}\``;
const list = (codes) => codes.map(code).join(", ");
const table = (header, rows) =>
  [
    `| ${header.join(" | ")} |`,
    `| ${header.map(() => "---").join(" | ")} |`,
    ...rows.map((r) => `| ${r.map(cell).join(" | ")} |`),
  ].join("\n");
const page = (title, description, body) =>
  `---\n${yaml.dump({ title, description }, { lineWidth: 100 }).trim()}\n---\n\n${body.trim()}\n`;
const firstSentence = (doc) => {
  const end = doc.search(/(?<!\b(?:i\.e|e\.g))\.\s/);
  return end === -1 ? doc : doc.slice(0, end + 1);
};
const num = (v, unit = "") => {
  if (v === undefined || v === null) return "—";
  const s = Number.isInteger(v) ? String(v) : v.toFixed(v < 10 ? 2 : 1);
  return unit ? `${s} ${unit}` : s;
};

// Doc comments are written for the operator's developers too: pointers
// into the code base or its design notes (`See \`a::b\``, `.prompt/…`)
// mean nothing on the site and are dropped from the meaning.
const devPointer = /::|\.prompt\/|\.rs\b/;
export function meaning(doc) {
  return doc
    .split(/\n\n/)
    .map((paragraph) =>
      paragraph
        .replace(/\s*\((?:[^()]*?)\)/g, (m) => (/^\s*\(see /i.test(m) && devPointer.test(m) ? "" : m))
        .replace(/\s*—\s*see [^()—]*?(?=\))/gi, (m) => (devPointer.test(m) ? "" : m))
        .split(/(?<=[.!?])\s+(?=[A-Z`(])/)
        .filter((sentence) => !(/^See\b/.test(sentence) && devPointer.test(sentence)))
        .join(" "),
    )
    .filter(Boolean)
    .join(" ");
}

/** Where a code shows up, from its severity and how the code uses it. */
export function shownAs(c, catalog, usage, overrides = {}) {
  if (overrides[c.code]) return overrides[c.code];
  if (c.code === catalog.success) return "`Ready: True`";
  if (c.severity === "Advisory") {
    if (usage.lifecycle.has(c.code) || usage.released.has(c.code) || /\bEvent only\b/i.test(c.doc)) {
      return "Event (`Normal`)";
    }
    return `condition \`${c.code}: True\`, Event (\`Normal\`)`;
  }
  const where = usage.errorsOnly.has(c.code)
    ? ["Event (`Warning`), before any status"]
    : ["`Ready: False`", "Event (`Warning`)"];
  if (usage.denials.has(c.code)) where.push("admission denial");
  return where.join(", ");
}

export function reasonCodesPage({ catalog, usage, remedies = {}, shownAs: overrides }) {
  const withRemedies = Object.keys(remedies).length > 0;
  for (const name of Object.keys({ ...remedies, ...overrides })) {
    if (!catalog.codes.some((c) => c.code === name)) {
      throw new Error(`reference config names ${name}, which is not a code of ${catalog.enumName}`);
    }
  }
  const rows = catalog.codes.map((c) => [
    code(c.code),
    c.severity === "Advisory" ? "advisory" : "blocking",
    shownAs(c, catalog, usage, overrides),
    meaning(c.doc),
    ...(withRemedies ? [remedies[c.code] ?? ""] : []),
  ]);
  const blocking = catalog.codes.find((c) => c.severity === "Blocking")?.code ?? "…";
  return page(
    "Reason codes",
    "Every code the operator puts on a condition, an Event or an admission denial.",
    `
Every condition, Event and admission denial the operator writes carries one
of these codes, never free text: alerts, scripts and \`kubectl\` filters can
rely on them. The list is generated from the operator's source, so it
matches the installed version.

- **Blocking**: \`Ready\` turns \`False\` with this reason, and every failed
  reconcile records a \`Warning\` Event with it ([Events](/docs/reference/events)).
  An admission webhook denial carries one too (HTTP 403, the code as its
  reason).
- **Advisory**: the CR still works. ${catalog.success ? `${code(catalog.success)} is the reason of \`Ready: True\`; the` : "The"}
  others are a condition of their own (its \`type\` is the code, its
  \`status\` \`"True"\`) or a lifecycle Event.

Every CR's \`Ready\` reason also labels the reconcile counter
([Metrics & health](/docs/reference/metrics)), so a code can be graphed and
alerted on. To find where one shows up right now:

\`\`\`bash
kubectl get events -A --field-selector reason=${blocking}
\`\`\`

${table(["Code", "Severity", "Shows as", "Meaning", ...(withRemedies ? ["What to do"] : [])], rows)}
`,
  );
}

export function eventsPage({ catalog, usage, group, runtime }) {
  const advisories = catalog.codes.filter(
    (c) => c.severity === "Advisory" && c.code !== catalog.success && shownAs(c, catalog, usage).startsWith("condition"),
  );
  const docOf = (name) => catalog.codes.find((c) => c.code === name)?.doc ?? "";
  const released = [...usage.released][0];
  const rows = [
    [
      "A reconcile ends on `Ready: False` — every time, not only the first",
      "`Warning`",
      "the failure's code",
      "`Reconcile`",
      "the condition's message",
    ],
    [
      "`Ready` turns `True`, or stays `True` with another reason",
      "`Normal`",
      catalog.success ? code(catalog.success) : "the success code",
      "`Reconcile`",
      "the condition's message",
    ],
    ...(advisories.length
      ? [
          ["An advisory condition appears or changes", "`Normal`", list(advisories.map((c) => c.code)), "`Reconcile`", "its message"],
          [
            "An advisory condition goes away",
            "`Normal`",
            "its code",
            "`Reconcile`",
            "`<type> no longer applies: <message>`",
          ],
        ]
      : []),
    ...[...usage.lifecycle].map(([name, action]) => [
      firstSentence(meaning(docOf(name))).replace(/\s*\(Event only[^)]*\)\.?/i, "."),
      "`Normal`",
      code(name),
      code(action),
      "what happened, with the remote object's id",
    ]),
    ...(usage.errors.size
      ? [
          [
            "A reconcile stops before writing a status (Kubernetes API call, finalizer…)",
            "`Warning`",
            list([...usage.errors.keys()]),
            "`Reconcile`",
            "the error",
          ],
        ]
      : []),
    ["A finalizer cleanup fails", "`Warning`", "the failure's code", "`Cleanup`", "the error"],
    ...(released
      ? [
          ["The finalizer finishes its cleanup", "`Normal`", code(released), "`Cleanup`", "`cleanup finished, finalizer released`"],
          [
            `The CR is force-orphaned (\`${group}/force-orphan: "true"\`)`,
            "`Normal`",
            code(released),
            "`Cleanup`",
            `\`force-orphaned (${group}/force-orphan): released without cleanup\``,
          ],
        ]
      : []),
  ];
  return page(
    "Events",
    "The Kubernetes Events the operator records on its CRs, and how to read them.",
    `
The operator records a Kubernetes Event on a CR for every error it hits and
every change of its state, so \`kubectl describe\` and \`kubectl get events\`
tell a CR's story without the operator's logs. An Event's \`reason\` is
always one of the [reason codes](/docs/reference/reason-codes); its
\`action\` is \`Reconcile\`, or \`Cleanup\` for the finalizer's work when the
CR is deleted.

${table(["When", "Type", "Reason", "Action", "Note"], rows)}

## Repeats

A CR that keeps failing is retried about every ${runtime.retry_requeue_secs} s, and warns each
time. Identical Events (same reason, note and action) are folded into one
series whose count goes up, so a stuck CR shows one line, not hundreds, and
the apiserver isn't flooded. A note is cut at ${runtime.event_note_max_chars} characters.

Publishing an Event never fails a reconcile: when the apiserver refuses
one, the operator logs it and carries on.

## Reading them

\`\`\`bash
kubectl describe <kind> <name> -n <namespace>     # Events at the bottom
kubectl events -n <namespace> --for <kind>/<name>
kubectl get events -A --field-selector type=Warning
\`\`\`

The apiserver keeps Events for an hour by default (\`--event-ttl\`): for a
longer history, ship them with an event exporter, or alert on the
[reconcile metrics](/docs/reference/metrics), which carry the same codes.
`,
  );
}

export function metricsPage({ prefix, leaderGauge, extraMetrics = [], runtime }) {
  const p = prefix;
  const buckets = runtime.reconcile_duration_buckets_secs.join(", ");
  const rows = [
    [
      `\`${p}_reconcile_total\``,
      "counter",
      "`kind`, `result` (`success`/`error`: the severity of the `Ready` reason), `reason` (the `Ready` reason code)",
      "Reconcile runs",
    ],
    [`\`${p}_reconcile_duration_seconds\``, "histogram", "`kind`", `Reconcile run duration; buckets ${buckets} s`],
    [
      `\`${p}_webhook_decisions_total\``,
      "counter",
      "`kind`, `result` (`allow`/`deny`)",
      "Admission webhook decisions",
    ],
    ...(leaderGauge
      ? [[`\`${leaderGauge}\``, "gauge (0/1)", "none", "`1` on the replica holding the leader lease"]]
      : []),
    ...extraMetrics.map((m) => [`\`${m.name}\``, m.type, m.labels ?? "", m.description ?? ""]),
  ];
  return page(
    "Metrics & health",
    "What the operator exports, the probes it answers, and the settings of its runtime.",
    `
## Exporting

| Variable | Effect |
| --- | --- |
| \`WEEBO_METRICS_ADDR\` | Serves Prometheus metrics in plain HTTP on that address (e.g. \`0.0.0.0:9090\`, \`GET /metrics\`), on every replica: a standby shows up too, its leader gauge at 0 |
| \`OTEL_EXPORTER_OTLP_ENDPOINT\` | Also exports traces and metrics over OTLP/gRPC |
| \`RUST_LOG\` | Log verbosity (default \`info\`) |
| \`LOG_FORMAT=json\` | Logs as JSON lines |

## Metrics

${table(["Metric", "Type", "Labels", "What"], rows)}

\`result\` and \`reason\` come from the reason code the reconcile ended on,
so the label set stays bounded by the [catalog](/docs/reference/reason-codes).
For example:

\`\`\`text
# Share of failing reconciles, per kind
sum by (kind) (rate(${p}_reconcile_total{result="error"}[15m]))
  / sum by (kind) (rate(${p}_reconcile_total[15m]))

# Why they fail
sum by (reason) (rate(${p}_reconcile_total{result="error"}[15m]))

# Reconcile p95, per kind
histogram_quantile(0.95, sum by (kind, le) (rate(${p}_reconcile_duration_seconds_bucket[5m])))
${leaderGauge ? `\n# Nothing reconciles: no replica holds the lease\nsum(${leaderGauge}) < 1\n` : ""}\`\`\`

## Probes

Served on every replica, on the admission webhook's HTTPS port
(\`WEBHOOK_PORT\`, default ${runtime.webhook_port}):

| Path | Answers 200 when |
| --- | --- |
| \`/readyz\` (alias \`/healthz\`) | The process serves admission requests. It turns 503 as soon as shutdown starts, so the endpoints drain before the server stops |
| \`/livez\` | No component kept failing for ${runtime.liveness_threshold_secs} s in a row (a controller's watch erroring in a loop); a streak ends after ${runtime.failure_streak_gap_secs} s without a new failure. Restarting the pod is then the remedy |

## Runtime settings

| Setting | Default | What it does |
| --- | --- | --- |
| \`WEEBO_RECONCILE_CONCURRENCY\` | ${runtime.default_concurrency} | Reconciles running at once, per controller: a startup sweep can't flood the remote system or the apiserver |
| \`WEEBO_WATCH_NAMESPACE\` | the whole cluster | The namespaced kinds of this namespace only (chart: \`watch.namespace\`); the cluster-scoped kinds stay watched cluster-wide |
| \`WEEBO_WATCH_CLUSTER_KINDS\` | \`true\` | \`false\` (chart: \`watch.clusterKinds\`, with a namespace): full namespaced — no cluster-scoped kind is watched, objects use the LocalInstances of the namespace, the operator runs on Roles only |
| \`WEEBO_CROSS_NAMESPACE_REFS\` | \`false\` | \`true\` (chart: \`references.crossNamespace\`): a CR may reference a CR of another namespace that shares itself with it (\`spec.sharedWith\`); never with \`WEEBO_WATCH_NAMESPACE\` |
| \`WEEBO_WATCH_LABEL_SELECTOR\` | every object | A JSON \`LabelSelector\` (chart: \`watch.labelSelector\`): only the objects it matches exist for the operator, every kind included. It splits the work between releases, it is not a security boundary. An invalid selector stops the start |
| Requeue | ${runtime.steady_requeue_secs} s / ${runtime.retry_requeue_secs} s | Next reconcile after a success (drift check) / after a failure, ±20 % so CRs don't move in lockstep. A failure only a change of the CR can clear waits the long delay: the change triggers a reconcile by itself |
| Status patch | ${runtime.status_retries} retries | A status update the apiserver couldn't take (unavailable, 429, 5xx) is retried before the reconcile fails |
| \`LEASE_NAME\`, \`POD_NAMESPACE\` | | The leader lease. Only its holder reconciles; it lasts ${runtime.lease_ttl_secs} s, renewed every ${runtime.lease_renew_secs} s, and a leader that may have lost it exits (its pod restarts as a standby) |
| \`WEEBO_OPERATOR_POD_LABELS\` | set by the chart | The operator pod's own selector labels (a JSON object, \`nameOverride\` applied): what a NetworkPolicy the operator writes selects to let the operator in, when it writes any |
| \`WEEBO_SHUTDOWN_DRAIN_SECONDS\` | ${runtime.shutdown_drain_secs} | After SIGTERM, how long the webhook keeps answering while endpoints drain; in-flight requests then get ${runtime.graceful_shutdown_secs} s |
| \`WEBHOOK_TLS_CERT_FILE\`, \`WEBHOOK_TLS_KEY_FILE\` | \`/etc/webhook/certs/tls.{crt,key}\` | The webhook's certificate, re-read every ${runtime.tls_reload_secs} s: a rotation needs no restart |
`,
  );
}

export function budgetPage({ budget, overrides = {}, report, perfTask = "task perf", reportPath, note, scheduled = false }) {
  const m = report ? measuredOf(report) : undefined;
  const rows = LIMITS.filter(([key]) => budget[key] != null).map(([key, what, unit]) => {
    const limit = num(budget[key], unit);
    const row = [what, key in overrides ? `${limit} (operator's)` : limit];
    if (m) {
      const v = m.values[key];
      row.push(num(v, unit), v === undefined ? "" : v <= budget[key] ? "ok" : "**over**");
    }
    return row;
  });
  const wl = report ? m.workload : undefined;
  // The report dates itself (weebo-kit-perf ≥ 0.8); `note` adds to it.
  const when = report?.measured_at
    ? `on ${report.measured_at.slice(0, 10)}${report.kit_version ? ` with weebo-kit-perf ${report.kit_version}` : ""}`
    : undefined;
  const about = [when, note].filter(Boolean).join(", ");
  const measuredIntro = report
    ? `The last run committed with the docs (\`${reportPath}\`${about ? `, ${about}` : ""}) measured
${wl ? `${wl.objects}${wl.resource ? ` \`${wl.resource}\`` : " objects"} per load` : "its loads"}; each value is the worst phase for that limit.`
    : "No run has been committed with the docs yet: see below.";
  return page(
    "Resource budget",
    "What the operator may cost a cluster, and what it was last measured at.",
    `
What the operator costs is measured before it ships, so it can sit on a
small cluster next to others without surprises. The harness
(\`weebo-kit-perf\`) starts the operator's **release binary** against a real
apiserver (envtest), the way it runs in a pod, and measures each phase of a
run: an idle startup, a warm-up load, an idle \`baseline\`, loads of CRs
(created at once, changed, deleted), and a final idle window. Leaks
(memory, file descriptors, threads) are judged between the two idle
windows.

## Budget

The limits are the kit's small-cluster budget (a few operators on a
2-4 GiB node)${Object.keys(overrides).length ? ", with the operator's own where marked" : ""}. ${measuredIntro}

${table(["Limit", "Budget", ...(m ? ["Measured", ""] : [])], rows)}

## Measuring it again

\`\`\`bash
${perfTask}
\`\`\`

The run fails when a limit is exceeded and writes its report to
\`target/perf/\` (Markdown and JSON). \`PERF_OBJECTS\` and \`PERF_IDLE_SECS\`
size it; \`WEEBO_PERF_BASELINE=<a previous JSON>\` compares with an earlier
run; \`WEEBO_PERF_REPORT_ONLY=1\` reports without failing.${
      reportPath
        ? `${scheduled ? " A scheduled workflow (`.github/workflows/perf.yml`, the kit's) measures it again every week on a CI runner and opens a pull request with the new report and its comparison with this one." : ""} By hand, copy the JSON report to \`docs/${reportPath}\` and regenerate the docs.`
        : ""
    }
`,
  );
}

function measuredOf(report) {
  return { values: measured(report), workload: workload(report) };
}
