// CRD reference generator: from the operator's CRD YAML (what its `crdgen`
// produces via `CustomResourceExt::crd()`) to
//   - <contentDir>/<singular>.mdx — facts, an example, the spec field table
//     and the manifest builder (`<CrdForm kind=… />`), one page per kind;
//   - <contentDir>/index.mdx + meta.json — the section's table of kinds and
//     its sidebar order;
//   - <schemaDir>/<singular>.schema.json — the spec schema the form fetches,
//     and <schemaDir>/index.json — the kind list `<CrdBuilder />` offers.
// Both output directories are owned by the generator: their previous
// .mdx/.json files are removed first, so a deleted CRD leaves no page behind.
//
// Field `description`s trace back to the Rust `#[schemars(description =
// ...)]`/doc-comments on each `*Spec` struct — a CR field with no
// description here means the Rust source is missing one, which is a useful
// signal in review, not a bug in this generator.
//
// Everything operator-specific (guides, examples, sidebar order, intro text)
// comes from the operator's config file, see `CrdDocsConfig` below. With a
// `reference` section, the same run also writes the operator reference
// pages (src/reference/).

import fs from "node:fs";
import path from "node:path";
import * as yaml from "js-yaml";
import { generateReference } from "../reference/generate.mjs";

/**
 * @typedef {object} CrdDocsConfig
 * @property {string} [crdDir] CRD YAML directory. Default `../deploy/crd`.
 * @property {string} [contentDir] Generated pages. Default `content/docs/crds`.
 * @property {string} [schemaDir] Generated schemas. Default `public/crd-schemas`.
 * @property {string} [baseUrl] URL of the section. Default `/docs/crds`.
 * @property {string} [kindPrefix] Dropped from a kind to name its example
 *   (`<prefix>Repository` → `my-repository`). Default: none.
 * @property {string} [exampleNamespace] Namespace of namespaced examples. Default `team-a`.
 * @property {Record<string, Record<string, unknown>>} examples A minimal,
 *   realistic `spec` per kind. Required for every kind, and checked against
 *   its schema (every required field present, every key a real field), so a
 *   renamed field fails generation instead of shipping a stale example.
 * @property {Record<string, [title: string, href: string]>} [guides] The guide
 *   that explains each kind, linked from its reference page.
 * @property {string[]} [order] Sidebar order — the order you'd create things
 *   in, not the alphabet. Kinds not listed come last, alphabetically.
 * @property {string} [indexIntro] The paragraph above the index table
 *   (Markdown). Default: a generic one.
 * @property {import("../reference/generate.mjs").ReferenceConfig} [reference]
 *   When set, also generates the operator reference pages (reason codes,
 *   Events, metrics & health, resource budget) from the operator's sources.
 */

/** Identity, for editor completion in the operator's config file. */
export function defineConfig(/** @type {CrdDocsConfig} */ config) {
  return config;
}

const DEFAULT_INTRO = `Each page lists a kind's \`spec\` fields, shows a minimal example, and has a
form that builds a ready-to-apply manifest. These pages are generated from
the CRDs themselves, so they always match the installed version.`;

// A Kubernetes structural schema isn't plain JSON Schema — these extensions
// have no meaning for a generic client-side form/validator and would only
// confuse it. Stripped from the published schemas, not from the CRD itself.
const K8S_EXTENSION_KEYS = new Set([
  "x-kubernetes-preserve-unknown-fields",
  "x-kubernetes-int-or-string",
]);

export function stripK8sExtensions(node) {
  if (Array.isArray(node)) {
    return node.map(stripK8sExtensions);
  }
  if (node && typeof node === "object") {
    const out = {};
    for (const [key, value] of Object.entries(node)) {
      if (K8S_EXTENSION_KEYS.has(key)) continue;
      out[key] = stripK8sExtensions(value);
    }
    return out;
  }
  return node;
}

function fieldType(schema) {
  if (!schema) return "object";
  if (schema.type === "array") {
    return `${fieldType(schema.items)}[]`;
  }
  if (schema.type === "object" && !schema.properties) {
    return "object";
  }
  return schema.type ?? "object";
}

const escapeOutsideCode = (text) => text.replace(/[\\{}<>]/g, "\\$&");

/**
 * CRD text, made safe to paste into MDX: outside code spans, `\ { } < >` are
 * escaped, so no description can become a JS expression (evaluated at build
 * time) or a JSX element. Code spans (a backtick run closed by a run of the
 * same length, as in CommonMark) are left as written: MDX reads neither
 * inside them. With `cell`, `|` is escaped everywhere (GFM splits a cell even
 * inside code) and line breaks become spaces.
 */
export function mdxText(text, { cell = false } = {}) {
  let out = "";
  let rest = text;
  for (let open = rest.match(/`+/); open; open = rest.match(/`+/)) {
    const fence = open[0];
    const after = rest.slice(open.index + fence.length);
    const close = after.match(new RegExp(`(?<!\`)${fence}(?!\`)`));
    out += escapeOutsideCode(rest.slice(0, open.index));
    if (close) {
      out += fence + after.slice(0, close.index + fence.length);
      rest = after.slice(close.index + fence.length);
    } else {
      // An orphan run is literal text, never the start of a code span.
      out += fence;
      rest = after;
    }
  }
  out += escapeOutsideCode(rest);
  return cell ? out.replace(/\|/g, "\\|").replace(/\r?\n/g, " ") : out;
}

function renderFieldsTable(properties, required) {
  const requiredSet = new Set(required ?? []);
  const rows = Object.entries(properties ?? {}).map(([name, schema]) => {
    const type = fieldType(schema);
    const isRequired = requiredSet.has(name) ? "yes" : "no";
    const def =
      schema.default !== undefined ? `\`${JSON.stringify(schema.default)}\`` : "";
    const description = (schema.description ?? "").trim();
    const cells = [`\`${name}\``, `\`${type}\``, isRequired, def, description];
    return `| ${cells.map((c) => mdxText(c, { cell: true })).join(" | ")} |`;
  });
  return [
    "| Field | Type | Required | Default | Description |",
    "| --- | --- | --- | --- | --- |",
    ...rows,
  ].join("\n");
}

/**
 * The first sentence of a spec's description, for the frontmatter and the
 * index table. The source doc-comment is hand-wrapped at ~80 columns in Rust,
 * not at sentence boundaries — join it into one paragraph first, so the
 * summary doesn't cut off mid-sentence wherever the source happened to wrap.
 * Only the first paragraph counts, so a list or section that follows it never
 * bleeds into the summary; a lead-in colon becomes a period.
 */
export function shortDescription(description) {
  const paragraph = description.split(/\n\s*\n/)[0].replace(/\s*\n\s*/g, " ");
  // "i.e." / "e.g." are not sentence ends.
  const end = paragraph.search(/(?<!\b(?:i\.e|e\.g))\.\s/);
  return (end === -1 ? paragraph : paragraph.slice(0, end + 1)).replace(/:$/, ".");
}

function checkExample(kind, example, specSchema) {
  if (!example) throw new Error(`no example spec for ${kind} in the crd-docs config`);
  const props = specSchema.properties ?? {};
  for (const key of Object.keys(example)) {
    if (!(key in props)) throw new Error(`${kind} example: unknown field spec.${key}`);
  }
  for (const key of specSchema.required ?? []) {
    if (!(key in example)) throw new Error(`${kind} example: missing required spec.${key}`);
  }
}

const scopeLabel = (scope) => (scope === "Namespaced" ? "Namespaced" : "Cluster");
const codeList = (names) => names.map((n) => `\`${n}\``).join(", ");

function clean(dir, extensions) {
  for (const file of fs.readdirSync(dir)) {
    if (extensions.some((ext) => file.endsWith(ext))) fs.rmSync(path.join(dir, file));
  }
}

/**
 * Generates the CRD reference. Relative paths in `config` resolve against
 * `root` (the directory of the operator's config file, i.e. its docs site).
 * Returns the number of CRDs processed.
 *
 * @param {CrdDocsConfig} config
 * @param {{ root: string }} options
 */
export function generateCrdDocs(config, { root }) {
  const crdDir = path.resolve(root, config.crdDir ?? "../deploy/crd");
  const mdxOutDir = path.resolve(root, config.contentDir ?? "content/docs/crds");
  const schemaOutDir = path.resolve(root, config.schemaDir ?? "public/crd-schemas");
  const baseUrl = config.baseUrl ?? "/docs/crds";
  const prefix = new RegExp(`^${config.kindPrefix ?? ""}`);
  const exampleNamespace = config.exampleNamespace ?? "team-a";
  const guides = config.guides ?? {};
  const examples = config.examples ?? {};
  const order = config.order ?? [];

  fs.mkdirSync(mdxOutDir, { recursive: true });
  fs.mkdirSync(schemaOutDir, { recursive: true });
  clean(mdxOutDir, [".mdx", "meta.json"]);
  clean(schemaOutDir, [".json"]);

  const files = fs
    .readdirSync(crdDir)
    .filter((f) => f.endsWith(".yaml"))
    .sort();
  const index = [];
  const groups = new Set();

  for (const file of files) {
    const crd = yaml.load(fs.readFileSync(path.join(crdDir, file), "utf8"));
    const kind = crd.spec.names.kind;
    const singular = crd.spec.names.singular;
    const version = crd.spec.versions[0];
    const specSchema = version.schema.openAPIV3Schema.properties.spec;
    const apiVersion = `${crd.spec.group}/${version.name}`;
    const scope = crd.spec.scope;
    groups.add(crd.spec.group);

    const description = (specSchema.description ?? `${kind} spec.`).trim();
    const summary = shortDescription(description);
    const table = renderFieldsTable(specSchema.properties, specSchema.required);
    const shortNames = crd.spec.names.shortNames ?? [];
    const [guideTitle, guideHref] = guides[kind] ?? [];

    checkExample(kind, examples[kind], specSchema);
    const metadata = { name: `my-${kind.replace(prefix, "").toLowerCase()}` };
    if (scope === "Namespaced") metadata.namespace = exampleNamespace;
    const example = yaml
      .dump({ apiVersion, kind, metadata, spec: examples[kind] }, { lineWidth: 100 })
      .trim();

    const facts = [
      `- **API version:** \`${apiVersion}\``,
      `- **Scope:** ${scopeLabel(scope)}`,
      `- **Short names:** ${codeList(shortNames) || "none"}`,
      ...(guideTitle ? [`- **Guide:** [${guideTitle}](${guideHref})`] : []),
    ].join("\n");

    const frontmatter = yaml
      // `full`: the page drops its TOC and widens to fit the manifest builder's
      // form and preview side by side; prose keeps its 68ch measure regardless.
      // Backticks and bold are stripped: the description renders as plain text.
      .dump({ title: kind, description: summary.replace(/`|\*\*/g, ""), full: true })
      .trim();
    const mdx = `---\n${frontmatter}\n---\n\n${facts}\n\n${mdxText(description)}\n\n## Example\n\n\`\`\`yaml\n${example}\n\`\`\`\n\n## Spec fields\n\n${table}\n\n## Build a manifest\n\nAnswer the questions and fill in the fields: the YAML preview updates as you type, ready to copy or download and \`kubectl apply\`.\n\n<CrdForm kind="${kind}" />\n`;
    fs.writeFileSync(path.join(mdxOutDir, `${singular}.mdx`), mdx);

    // apiVersion/kind/scope aren't JSON Schema keywords — added here purely
    // for the client-side form (CrdForm) to assemble a full manifest
    // (apiVersion + kind + metadata.namespace-or-not) without a second
    // fetch. Consumers that want strict JSON Schema should ignore them.
    const cleanSchema = { apiVersion, kind, scope, ...stripK8sExtensions(specSchema) };
    fs.writeFileSync(
      path.join(schemaOutDir, `${singular}.schema.json`),
      `${JSON.stringify(cleanSchema, null, 2)}\n`,
    );

    index.push({ kind, singular, scope, shortNames, summary });
  }

  const rank = (kind) => (order.includes(kind) ? order.indexOf(kind) : order.length);
  index.sort((a, b) => rank(a.kind) - rank(b.kind) || a.kind.localeCompare(b.kind));

  // The short-names column only when some kind has one.
  const withShortNames = index.some((k) => k.shortNames.length > 0);
  const header = withShortNames
    ? "| Kind | Short names | Scope | Description |\n| --- | --- | --- | --- |"
    : "| Kind | Scope | Description |\n| --- | --- | --- |";
  const rows = index.map((k) =>
    [
      `[\`${k.kind}\`](${baseUrl}/${k.singular})`,
      ...(withShortNames ? [codeList(k.shortNames)] : []),
      scopeLabel(k.scope),
      mdxText(k.summary, { cell: true }),
    ].join(" | "),
  );
  const indexMdx = `---
title: CRD reference
description: Every kind the operator adds, its fields, and an interactive manifest builder.
---

${(config.indexIntro ?? DEFAULT_INTRO).trim()}

${header}
${rows.map((r) => `| ${r} |`).join("\n")}
`;
  fs.writeFileSync(path.join(mdxOutDir, "index.mdx"), indexMdx);
  fs.writeFileSync(
    path.join(mdxOutDir, "meta.json"),
    `${JSON.stringify({ title: "CRD reference", pages: ["index", ...index.map((k) => k.singular)] }, null, 2)}\n`,
  );
  // The kind picker of `<CrdBuilder />` reads this: every CRD, without a
  // second data source to keep in sync.
  fs.writeFileSync(
    path.join(schemaOutDir, "index.json"),
    `${JSON.stringify(index.map(({ kind, singular, scope }) => ({ kind, singular, scope })), null, 2)}\n`,
  );

  if (config.reference) {
    if (groups.size !== 1) throw new Error(`the CRDs span ${groups.size} API groups, the reference needs one`);
    generateReference(config.reference, { root, group: [...groups][0] });
  }

  return files.length;
}
