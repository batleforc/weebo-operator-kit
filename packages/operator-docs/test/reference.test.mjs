import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { generateCrdDocs } from "../src/crd-docs/generate.mjs";
import { meaning, shownAs } from "../src/reference/pages.mjs";
import { measured, workload } from "../src/reference/perf.mjs";
import { parseReasonCodes, scanUsage } from "../src/reference/rust.mjs";

const fixtures = path.join(path.dirname(fileURLToPath(import.meta.url)), "fixtures");
const catalogFile = path.join(fixtures, "crates/domain/src/error.rs");
const catalog = parseReasonCodes(fs.readFileSync(catalogFile, "utf8"));
const usage = scanUsage(path.join(fixtures, "crates"), catalog.enumName, catalogFile);

test("the reason_codes! catalog: codes, docs, severities, success", () => {
  assert.equal(catalog.enumName, "ReasonCode");
  assert.equal(catalog.success, "Reconciled");
  assert.deepEqual(
    catalog.codes.map((c) => `${c.code}:${c.severity}`),
    [
      "Reconciled:Advisory",
      "Unreachable:Blocking",
      "NotAllowed:Blocking",
      "MemberPending:Advisory",
      "Created:Advisory",
      "Released:Advisory",
      "KubernetesApiError:Blocking",
    ],
  );
  assert.equal(
    catalog.codes[1].doc,
    "The remote object is unreachable.\n\nRetried. See `application::ports::GatewayError`.",
  );
  assert.equal(catalog.codes[2].doc, "Refused by the allow-list.");
  assert.throws(() => parseReasonCodes("pub enum X { A }"), /no reason_codes!/);
});

test("how the code uses it, tests left out", () => {
  assert.deepEqual([...usage.denials], ["NotAllowed"]);
  assert.deepEqual([...usage.lifecycle], [["Created", "Reconcile"]]);
  assert.deepEqual([...usage.released], ["Released"]);
  assert.deepEqual([...usage.errors.keys()], ["KubernetesApiError", "Unreachable"]);
  // Unreachable is also a Failure: it reaches a condition.
  assert.deepEqual([...usage.errorsOnly], ["KubernetesApiError"]);
});

test("where each code shows up", () => {
  const where = Object.fromEntries(catalog.codes.map((c) => [c.code, shownAs(c, catalog, usage)]));
  assert.equal(where.Reconciled, "`Ready: True`");
  assert.equal(where.Unreachable, "`Ready: False`, Event (`Warning`)");
  assert.equal(where.NotAllowed, "`Ready: False`, Event (`Warning`), admission denial");
  assert.equal(where.MemberPending, "condition `MemberPending: True`, Event (`Normal`)");
  assert.equal(where.Created, "Event (`Normal`)");
  assert.equal(where.Released, "Event (`Normal`)");
  assert.equal(where.KubernetesApiError, "Event (`Warning`), before any status");
});

test("pointers into the code base are dropped from a meaning", () => {
  assert.equal(meaning(catalog.codes[1].doc), "The remote object is unreachable. Retried.");
  assert.equal(meaning(catalog.codes[3].doc), "A member is missing; the rest syncs.");
});

test("a perf report, checked like Report::check", () => {
  const report = JSON.parse(fs.readFileSync(path.join(fixtures, "perf/weebo-example.json"), "utf8"));
  const m = measured(report);
  assert.equal(m.idle_rss_mib, 36);
  assert.equal(m.cpu_ms_per_object, 10);
  assert.equal(m.api_requests_per_object, 6);
  assert.equal(m.convergence_secs_per_100, 2);
  assert.equal(m.rss_growth_pct, 20);
  assert.equal(m.fd_growth, 2);
  assert.deepEqual(workload(report), { objects: 100, resource: "widgets" });
});

const crdConfig = {
  crdDir: path.join(fixtures, "crd"),
  examples: { ExampleWidget: { name: "w" }, ExampleGadget: { url: "https://g" } },
};
const reference = {
  reasonCodes: catalogFile,
  sources: path.join(fixtures, "crates"),
  metricsPrefix: "weebo_example",
  extraMetrics: [{ name: "weebo_example_remote_calls", type: "counter", labels: "`op`" }],
  remedies: { Unreachable: "Check the URL" },
  perfReport: path.join(fixtures, "perf/weebo-example.json"),
};

test("the four reference pages, from the operator's sources", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "reference-"));
  generateCrdDocs({ ...crdConfig, reference }, { root });
  const read = (p) => fs.readFileSync(path.join(root, "content/docs/reference", p), "utf8");
  assert.deepEqual(JSON.parse(read("meta.json")).pages, [
    "reason-codes",
    "events",
    "metrics",
    "resource-budget",
    "...",
  ]);
  const codes = read("reason-codes.mdx");
  assert.match(codes, /\| `Unreachable` \| blocking \| `Ready: False`, Event \(`Warning`\) \| The remote object is unreachable\. Retried\. \| Check the URL \|/);
  const events = read("events.mdx");
  assert.match(events, /`example\.weebo\.io\/force-orphan: "true"`/);
  assert.match(events, /\| The remote object was created\. \| `Normal` \| `Created` \| `Reconcile` \|/);
  assert.match(events, /`KubernetesApiError`, `Unreachable`/);
  const metrics = read("metrics.mdx");
  assert.match(metrics, /`weebo_example_reconcile_total`/);
  assert.match(metrics, /`weebo_example_leader`/);
  assert.match(metrics, /`weebo_example_remote_calls` \| counter \| `op`/);
  const budget = read("resource-budget.mdx");
  assert.match(budget, /\| Memory at rest \(median RSS of every idle window\) \| 64 MiB \| 36 MiB \| ok \|/);
  assert.match(budget, /\| Memory not given back: final vs baseline idle window \| 10 % \| 20 % \| \*\*over\*\* \|/);
  assert.match(budget, /100 `widgets` per load/);
  fs.rmSync(root, { recursive: true });
});

test("names that don't match the sources fail generation", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "reference-"));
  const run = (r) => generateCrdDocs({ ...crdConfig, reference: { ...reference, ...r } }, { root });
  assert.throws(() => run({ metricsPrefix: "weebo_other" }), /metric prefix "weebo_other" not found/);
  assert.throws(() => run({ extraMetrics: [{ name: "weebo_gone", type: "counter" }] }), /metric weebo_gone not found/);
  assert.throws(() => run({ remedies: { Gone: "x" } }), /names Gone, which is not a code/);
  assert.throws(() => run({ budget: { idle_ram: 1 } }), /not a budget field/);
  fs.rmSync(root, { recursive: true });
});
