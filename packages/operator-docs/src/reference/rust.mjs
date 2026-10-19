// Reads what the reference pages need straight from the operator's Rust
// sources, so they can't drift from the code: the `reason_codes!` catalog
// (codes, doc comments, severities, the success code) and how the code
// uses it (admission denials, lifecycle Events, the finalizer's release
// code, the errors announced before a status). A text scan, not a Rust
// parser: it relies on the kit's conventions (`reason_codes!`, `Severity`,
// `Denial::new`, `events::announce_change`, `controller::finalize`,
// `(Code, events::RECONCILE|CLEANUP)`), and fails loudly when the catalog
// can't be found.

import fs from "node:fs";
import path from "node:path";

/** The body between the brace at `open` and its matching closing brace. */
function braced(source, open) {
  let depth = 0;
  for (let i = open; i < source.length; i++) {
    if (source[i] === "{") depth++;
    else if (source[i] === "}" && --depth === 0) return source.slice(open + 1, i);
  }
  throw new Error("unbalanced braces");
}

/** A `///` doc comment as one Markdown paragraph block (paragraphs kept). */
function docText(lines) {
  return lines
    .join("\n")
    .split(/\n\s*\n/)
    .map((p) => p.replace(/\s*\n\s*/g, " ").trim())
    .filter(Boolean)
    .join("\n\n");
}

/**
 * The catalog declared with the kit's `reason_codes!` in `source`:
 * `{ enumName, success, codes: [{ code, doc, severity }] }`, in
 * declaration order. `severity` is "Advisory" or "Blocking", read from the
 * `fn severity` that matches on the codes (unmatched codes take its `_`
 * arm, Blocking when there is none).
 */
export function parseReasonCodes(source) {
  const macro = source.search(/reason_codes!\s*\{/);
  if (macro === -1) throw new Error("no reason_codes! invocation found");
  const body = braced(source, source.indexOf("{", macro));
  const enumMatch = /enum\s+(\w+)\s*\{/.exec(body);
  if (!enumMatch) throw new Error("reason_codes! without an enum");
  const enumName = enumMatch[1];
  const variants = braced(body, enumMatch.index + enumMatch[0].length - 1);

  const codes = [];
  let doc = [];
  let attribute = 0;
  for (const raw of variants.split("\n")) {
    const line = raw.trim();
    if (attribute > 0 || line.startsWith("#[")) {
      // Attributes may span lines: skip until their brackets close.
      attribute += (line.match(/\[/g) ?? []).length - (line.match(/\]/g) ?? []).length;
      continue;
    }
    if (line.startsWith("///")) {
      doc.push(line.replace(/^\/\/\/ ?/, ""));
      continue;
    }
    if (line === "" || line.startsWith("//")) continue;
    const m = /^([A-Z]\w*)\s*,?$/.exec(line);
    if (!m) throw new Error(`unexpected line in ${enumName}: ${line}`);
    codes.push({ code: m[1], doc: docText(doc), severity: "Blocking" });
    doc = [];
  }
  if (codes.length === 0) throw new Error(`${enumName} declares no code`);

  // The `fn severity` that matches on the codes (not the trait impl that
  // delegates to it).
  let fallback = "Blocking";
  const severities = new Map();
  for (const fn of source.matchAll(/fn\s+severity\s*\([^)]*\)\s*->\s*Severity\s*\{/g)) {
    const fnBody = braced(source, fn.index + fn[0].length - 1);
    if (!/\bmatch\b/.test(fnBody)) continue;
    const arm = new RegExp(`((?:\\|?\\s*${enumName}::\\w+\\s*)+)=>\\s*Severity::(\\w+)`, "g");
    for (const [, patterns, severity] of fnBody.matchAll(arm)) {
      for (const [, code] of patterns.matchAll(new RegExp(`${enumName}::(\\w+)`, "g"))) {
        severities.set(code, severity);
      }
    }
    const wildcard = /_\s*=>\s*Severity::(\w+)/.exec(fnBody);
    if (wildcard) fallback = wildcard[1];
  }
  for (const c of codes) c.severity = severities.get(c.code) ?? fallback;

  const success = new RegExp(`const\\s+SUCCESS\\s*:\\s*Self\\s*=\\s*${enumName}::(\\w+)`).exec(source);
  return { enumName, success: success?.[1], codes };
}

function rustFiles(dir) {
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      // Tests and build output say nothing about what the operator emits.
      if (["target", "tests", "test_support", "testkit"].includes(entry.name)) continue;
      out.push(...rustFiles(full));
    } else if (entry.name.endsWith(".rs") && entry.name !== "test_support.rs") {
      out.push(full);
    }
  }
  return out;
}

/** `#[cfg(test)] mod … { … }` blocks removed: unit tests aren't the operator. */
function withoutTestModules(source) {
  let out = source;
  for (;;) {
    const m = /#\[cfg\(test\)\]\s*mod\s+\w+\s*\{/.exec(out);
    if (!m) return out;
    const body = braced(out, m.index + m[0].length - 1);
    out = out.slice(0, m.index) + out.slice(m.index + m[0].length + body.length + 1);
  }
}

/**
 * The operator's leader gauge, `<prefix>_leader`, when its sources register
 * it: by name, or through the kit's `leader::register_leader_gauge`.
 */
export function leaderGauge(text, prefix) {
  return text.includes(`"${prefix}_leader"`) || /\bregister_leader_gauge\s*\(/.test(text)
    ? `${prefix}_leader`
    : undefined;
}

/**
 * Whether the operator counts its writes to the remote system: its sources
 * call the kit's `ReconcileMetrics::record_write` (or name
 * `<prefix>_remote_writes`). Without it, `<prefix>_remote_writes_total` is
 * never exported (an operator that only applies Kubernetes objects).
 */
export function countsWrites(text, prefix) {
  return text.includes(`"${prefix}_remote_writes"`) || /\brecord_write\s*\(/.test(text);
}

/**
 * How the operator's code uses its catalog, from every non-test `.rs` file
 * under `dir` but `catalogFile`:
 * - `denials`: codes of an admission `Denial` (or `undecidable`);
 * - `lifecycle`: code → action (`Reconcile`/`Cleanup`) of an
 *   `announce_change` (or `announce_drift`) Event;
 * - `released`: the code the kit's `controller::finalize` (or
 *   `finalize_with`) announces;
 * - `errors`: code → actions of the `(Code, events::…)` pairs an
 *   `error_policy` warns with, and `errorsOnly`: those never used
 *   anywhere else (they never reach a condition);
 * - `text`: the scanned sources (to check that a metric name exists).
 */
export function scanUsage(dir, enumName, catalogFile) {
  const files = rustFiles(dir).filter((f) => path.resolve(f) !== path.resolve(catalogFile ?? ""));
  const text = files.map((f) => withoutTestModules(fs.readFileSync(f, "utf8"))).join("\n");
  const code = `(?:\\w+::)*${enumName}::(\\w+)`;
  const action = `events::(RECONCILE|CLEANUP)`;
  const label = { RECONCILE: "Reconcile", CLEANUP: "Cleanup" };
  const all = (re) => [...text.matchAll(re)];
  const codes = (re) => new Set(all(re).map((m) => m[1]));

  const lifecycle = new Map();
  for (const [, c, a] of all(new RegExp(`announce_change\\([^;]*?${code}\\s*,\\s*${action}`, "g"))) {
    lifecycle.set(c, label[a]);
  }
  // The kit's `announce_drift`: always on a reconcile.
  for (const [, c] of all(new RegExp(`announce_drift\\([^;]*?${code}`, "g"))) {
    lifecycle.set(c, label.RECONCILE);
  }
  const errors = new Map();
  for (const [, c, a] of all(new RegExp(`\\(\\s*${code}\\s*,\\s*${action}\\s*\\)`, "g"))) {
    errors.set(c, new Set([...(errors.get(c) ?? []), label[a]]));
  }
  const mentions = (c) => all(new RegExp(`${enumName}::${c}\\b`, "g")).length;
  const errorsOnly = new Set(
    [...errors.keys()].filter(
      (c) => mentions(c) === all(new RegExp(`\\(\\s*(?:\\w+::)*${enumName}::${c}\\s*,\\s*${action}`, "g")).length,
    ),
  );
  return {
    denials: new Set([
      ...codes(new RegExp(`Denial::new\\(\\s*${code}`, "g")),
      ...codes(new RegExp(`fn\\s+undecidable\\s*\\([^)]*\\)[^{]*\\{\\s*${code}`, "g")),
    ]),
    lifecycle,
    released: codes(new RegExp(`controller::finalize(?:_with)?\\([^;]*?${code}`, "g")),
    errors,
    errorsOnly,
    text,
  };
}
