// The operator reference pages (reason codes, Events, metrics & health,
// resource budget), generated next to the CRD pages from the operator's own
// sources: its `reason_codes!` catalog and the code using it, its metric
// prefix, its CRDs' group, and the `weebo-kit-perf` report it committed.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { budgetPage, eventsPage, metricsPage, reasonCodesPage } from "./pages.mjs";
import { SMALL_CLUSTER } from "./perf.mjs";
import { countsWrites, leaderGauge as leaderGaugeOf, parseReasonCodes, scanUsage } from "./rust.mjs";

/** The kit runtime's constants the pages quote, kept equal by a weebo-kit-runtime test. */
const RUNTIME = JSON.parse(
  fs.readFileSync(fileURLToPath(new URL("./kit-runtime.json", import.meta.url)), "utf8"),
);

export const PAGES = ["reason-codes", "events", "metrics", "resource-budget"];

/**
 * @typedef {object} ReferenceConfig
 * @property {string} metricsPrefix The operator's metric prefix (`weebo_<name>`),
 *   as passed to the kit's `ReconcileMetrics`/`admission::router`. Checked
 *   against the sources.
 * @property {string} [group] The API group the pages name (the domain of
 *   the operator's annotations). Default: the CRDs' group. Required when
 *   there is no CRD; must match the CRDs' group otherwise.
 * @property {string} [dir] Generated pages. Default `content/docs/reference`.
 * @property {string} [reasonCodes] The file declaring the `reason_codes!`
 *   catalog. Default `../crates/domain/src/error.rs`.
 * @property {string} [sources] Rust sources scanned for how the codes are
 *   used. Default `../crates`.
 * @property {Record<string, string>} [remedies] "What to do" per code (Markdown).
 * @property {Record<string, string>} [shownAs] Where a code shows up, when
 *   the scan gets it wrong.
 * @property {{ name: string, type: string, labels?: string, description?: string }[]} [extraMetrics]
 *   The operator's own metrics, beyond the kit's. Each name is checked
 *   against the sources.
 * @property {Record<string, number>} [budget] Overrides of the kit's
 *   small-cluster budget, as the operator's perf test sets them.
 * @property {string} [perfReport] A `weebo-kit-perf` JSON report committed
 *   with the docs (`target/perf/<label>.json`, copied).
 * @property {string} [perfNote] What that run was, beyond the date and kit
 *   version the report records itself (the machine…).
 * @property {boolean} [perfScheduled] The kit's `perf.yml` workflow refreshes
 *   the report on a schedule.
 * @property {string} [perfTask] The command that runs it. Default `task perf`.
 */

/**
 * Writes the reference pages and their `meta.json` under `root`
 * (relative paths resolve against it). `group`: the CRDs' API group, the
 * domain of the operator's annotations.
 *
 * @param {ReferenceConfig} config
 * @param {{ root: string, group: string }} options
 */
export function generateReference(config, { root, group }) {
  if (!config.metricsPrefix) throw new Error("reference.metricsPrefix is required");
  const dir = path.resolve(root, config.dir ?? "content/docs/reference");
  const catalogFile = path.resolve(root, config.reasonCodes ?? "../crates/domain/src/error.rs");
  const catalog = parseReasonCodes(fs.readFileSync(catalogFile, "utf8"));
  const usage = scanUsage(path.resolve(root, config.sources ?? "../crates"), catalog.enumName, catalogFile);

  const prefix = config.metricsPrefix;
  if (!usage.text.includes(`"${prefix}"`)) {
    throw new Error(`metric prefix "${prefix}" not found in the operator's sources`);
  }
  for (const m of config.extraMetrics ?? []) {
    if (!usage.text.includes(m.name)) throw new Error(`metric ${m.name} not found in the operator's sources`);
  }
  const leaderGauge = leaderGaugeOf(usage.text, prefix);
  const remoteWrites = countsWrites(usage.text, prefix);

  const overrides = config.budget ?? {};
  for (const key of Object.keys(overrides)) {
    if (!(key in SMALL_CLUSTER)) throw new Error(`reference.budget.${key} is not a budget field`);
  }
  const report = config.perfReport
    ? JSON.parse(fs.readFileSync(path.resolve(root, config.perfReport), "utf8"))
    : undefined;

  const pages = {
    "reason-codes": reasonCodesPage({ catalog, usage, remedies: config.remedies, shownAs: config.shownAs }),
    events: eventsPage({ catalog, usage, group, runtime: RUNTIME }),
    metrics: metricsPage({ prefix, leaderGauge, remoteWrites, extraMetrics: config.extraMetrics, runtime: RUNTIME }),
    "resource-budget": budgetPage({
      budget: { ...SMALL_CLUSTER, ...overrides },
      overrides,
      report,
      perfTask: config.perfTask,
      reportPath: config.perfReport,
      note: config.perfNote,
      scheduled: config.perfScheduled,
    }),
  };
  fs.mkdirSync(dir, { recursive: true });
  for (const [name, mdx] of Object.entries(pages)) {
    fs.writeFileSync(path.join(dir, `${name}.mdx`), mdx);
  }
  // `...`: pages the operator adds to the section by hand come after.
  fs.writeFileSync(
    path.join(dir, "meta.json"),
    `${JSON.stringify({ title: "Operator reference", pages: [...PAGES, "..."] }, null, 2)}\n`,
  );
  return catalog.codes.length;
}
