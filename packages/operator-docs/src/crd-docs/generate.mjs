// CRD reference generator: from the operator's CRD YAML (what its `crdgen`
// produces via `CustomResourceExt::crd()`) to
//   - <contentDir>/<singular>.mdx — facts, an example, the spec field table
//     with a section per nested object (RFC 0004), and the manifest builder
//     (`<CrdForm kind=… />`), one page per kind;
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

// A map is an object with no `properties` and a schema for its values.
const mapValues = (schema) =>
  schema?.type === "object" && !schema.properties && typeof schema.additionalProperties === "object"
    ? schema.additionalProperties
    : undefined;

function fieldType(schema) {
  if (!schema) return "object";
  if (schema["x-kubernetes-int-or-string"]) return "int-or-string";
  if (schema.type === "array") {
    return `${fieldType(schema.items)}[]`;
  }
  const values = mapValues(schema);
  if (values) return `map[${fieldType(values)}]`;
  if (schema.type === "object" && !schema.properties) {
    return "object";
  }
  return schema.type ?? "object";
}

/**
 * Arrays and maps unwrapped down to what they hold: `suffix` is what they add
 * to the path (`[]`, `[key]`), `wrappers` the nodes crossed on the way.
 */
function unwrap(schema) {
  const wrappers = [];
  let suffix = "";
  let inner = schema;
  for (;;) {
    if (inner?.type === "array" && inner.items) {
      wrappers.push([suffix, inner]);
      suffix += "[]";
      inner = inner.items;
    } else if (mapValues(inner)) {
      wrappers.push([suffix, inner]);
      suffix += "[key]";
      inner = mapValues(inner);
    } else {
      return { suffix, inner, wrappers };
    }
  }
}

/** An object gets its own section when it has fields, at the end of any arrays and maps. */
const hasSection = (schema) => Object.keys(unwrap(schema).inner?.properties ?? {}).length > 0;

/**
 * `oneOf`/`anyOf` as serde writes an externally tagged enum: each branch only
 * requires one field. `names` is undefined for any other shape.
 */
function unionOf(schema) {
  for (const key of ["oneOf", "anyOf"]) {
    const branches = schema?.[key];
    if (!branches?.length) continue;
    const simple = branches.every((b) => Object.keys(b).length === 1 && b.required?.length === 1);
    return { key, count: branches.length, names: simple ? branches.map((b) => b.required[0]) : undefined };
  }
  return undefined;
}

const PROSE_ESCAPES = /[\\{}<>]|!(?=\[)/g;
const escapeOutsideCode = (text) => text.replace(PROSE_ESCAPES, "\\$&");

/** Code spans left as written, the rest escaped; spans never cross `text`. */
function escapeParagraph(text) {
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
      // An orphan run is literal text, and escaped: in a cell, paragraphs end
      // up on one line, where it could pair with a run of a later paragraph.
      out += fence.replace(/`/g, "\\`");
      rest = after;
    }
  }
  return out + escapeOutsideCode(rest);
}

/**
 * CRD prose, made safe to paste into MDX: outside code spans, `\ { } < >` and
 * the `!` of `![` are escaped, so no description can become a JS expression
 * (evaluated at build time), a JSX element or an image fetched at build time.
 * Code spans (a backtick run closed by a run of the same length, as in
 * CommonMark) are left as written: MDX reads neither inside them. The text is
 * cut into paragraphs first, because no code span crosses a blank line. With
 * `cell`, `|` is escaped everywhere (GFM splits a cell even inside code), a
 * paragraph break becomes `<br /><br />` and a line break a space.
 */
export function mdxText(text, { cell = false } = {}) {
  const parts = text.split(/(\r?\n[ \t]*\r?\n\s*)/);
  const out = parts.map((part, i) => (i % 2 ? (cell ? "<br /><br />" : part) : escapeParagraph(part)));
  return cell ? out.join("").replace(/\|/g, "\\|").replace(/\r?\n/g, " ") : out.join("");
}

/**
 * CRD text shown as code (names, defaults, enum values, rules): a fence
 * longer than any backtick run inside, padded when the text starts or ends
 * with a backtick or a space, line breaks joined (as CommonMark does), and
 * `|` escaped in a cell. Nothing inside a code span is read as MDX.
 */
export function codeSpan(text, { cell = false } = {}) {
  const flat = String(text).replace(/\s*\r?\n\s*/g, " ");
  const longest = Math.max(0, ...(flat.match(/`+/g) ?? []).map((run) => run.length));
  const fence = "`".repeat(longest + 1);
  const pad = /^[ `]|[ `]$/.test(flat) ? " " : "";
  const span = `${fence}${pad}${flat}${pad}${fence}`;
  return cell ? span.replace(/\|/g, "\\|") : span;
}

const CONSTRAINTS = [
  "pattern",
  "minimum",
  "exclusiveMinimum",
  "maximum",
  "exclusiveMaximum",
  "minLength",
  "maxLength",
  "minItems",
  "maxItems",
];

/** The `Values:` and `Constraints:` fragments of a field's description cell. */
function cellFragments(schema) {
  const fragments = [];
  const values = (schema.enum ?? unwrap(schema).inner?.enum ?? []).filter((v) => v !== null);
  if (values.length) {
    fragments.push(`Values: ${values.map((v) => codeSpan(JSON.stringify(v), { cell: true })).join(", ")}`);
  }
  const bounds = CONSTRAINTS.filter((key) => schema[key] !== undefined).map((key) =>
    codeSpan(`${key}: ${typeof schema[key] === "string" ? schema[key] : JSON.stringify(schema[key])}`, { cell: true }),
  );
  if (bounds.length) fragments.push(`Constraints: ${bounds.join(", ")}`);
  return fragments;
}

const ANCHOR_CHARS = /^[A-Za-z0-9_.-]+$/;
const anchorOf = (fieldPath) => fieldPath.replace(/\[(key)?\]/g, "");

/**
 * The Markdown of one object: its field table, then its union and its rules.
 * `ctx` carries the page's anchors (path → section) and its count of fields
 * without a description.
 */
function renderObject(schema, fieldPath, ctx) {
  const properties = schema.properties ?? {};
  const required = new Set(schema.required ?? []);
  const union = unionOf(schema);
  const choice = union?.names && union.names.length > 1 ? new Set(union.names) : new Set();
  const choiceLabel = union?.key === "anyOf" ? "any of" : "one of";
  // A one-branch union is no choice: its field is just required.
  if (union?.names?.length === 1) required.add(union.names[0]);

  const rules = renderRules(schema["x-kubernetes-validations"]);
  const rows = Object.entries(properties).map(([name, field]) => {
    if (!ANCHOR_CHARS.test(name)) {
      throw new Error(`${ctx.kind}: field ${fieldPath}.${JSON.stringify(name)} is not in [A-Za-z0-9_.-]`);
    }
    const childPath = `${fieldPath}.${name}`;
    const nameCell = hasSection(field)
      ? `[${codeSpan(name, { cell: true })}](#${anchorOf(childPath)})`
      : codeSpan(name, { cell: true });
    const isRequired = choice.has(name) ? choiceLabel : required.has(name) ? "yes" : "no";
    const def = field.default !== undefined ? codeSpan(JSON.stringify(field.default), { cell: true }) : "";
    const description = (field.description ?? "").trim();
    if (!description) ctx.undescribed += 1;
    const text = [description && mdxText(description, { cell: true }), ...cellFragments(field)].filter(Boolean);
    // Rules on a field with no section of its own (a string, a list) are
    // listed here, under the object that holds the field.
    const { suffix, inner, wrappers } = unwrap(field);
    for (const [at, node] of hasSection(field) ? wrappers : [...wrappers, [suffix, inner]]) {
      rules.push(...renderRules(node["x-kubernetes-validations"], `${name}${at}`));
    }
    return `| ${nameCell} | ${codeSpan(fieldType(field), { cell: true })} | ${isRequired} | ${def} | ${text.join("<br /><br />")} |`;
  });

  const parts = [
    ["| Field | Type | Required | Default | Description |", "| --- | --- | --- | --- | --- |", ...rows].join("\n"),
  ];
  if (union?.names && union.names.length > 1) {
    const lead = union.key === "anyOf" ? "At least one of" : "Exactly one of";
    parts.push(`${lead} ${union.names.map((n) => codeSpan(n)).join(", ")}.`);
  } else if (union && !union.names) {
    const lead = union.key === "anyOf" ? "Matches at least one of" : "Matches one of";
    parts.push(`${lead} ${union.count} shapes; see the schema.`);
  }
  if (rules.length) parts.push(`**Rules**\n\n${rules.join("\n")}`);
  return parts.join("\n\n");
}

/** One list item per CEL rule: its message, the rule, and its fieldPath. */
function renderRules(validations, field) {
  return (validations ?? []).map(({ message, rule, fieldPath }) => {
    const parts = [
      ...(field ? [`${codeSpan(field)}:`] : []),
      ...(message ? [`${mdxText(message.replace(/\s*\r?\n\s*/g, " "))} —`] : []),
      codeSpan(rule),
      ...(fieldPath ? [`(on ${codeSpan(fieldPath)})`] : []),
    ];
    return `- ${parts.join(" ")}`;
  });
}

/** A section per descendant object, depth first, in declaration order. */
function renderSections(schema, fieldPath, ctx) {
  const sections = [];
  for (const [name, field] of Object.entries(schema.properties ?? {})) {
    if (!hasSection(field)) continue;
    const { suffix, inner } = unwrap(field);
    const childPath = `${fieldPath}.${name}${suffix}`;
    const anchor = anchorOf(childPath);
    if (ctx.anchors.has(anchor)) {
      throw new Error(`${ctx.kind}: ${ctx.anchors.get(anchor)} and ${childPath} would share the anchor #${anchor}`);
    }
    ctx.anchors.set(anchor, childPath);
    sections.push(`### ${codeSpan(childPath)} [#${anchor}]\n\n${renderObject(inner, childPath, ctx)}`);
    sections.push(...renderSections(inner, childPath, ctx));
  }
  return sections;
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
const codeList = (names, options) => names.map((n) => codeSpan(n, options)).join(", ");

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

  // Written only once every page is built: a refused CRD leaves the previous
  // pages in place, not half of a new set.
  const writes = [];

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
    // The apiserver checks both, but this reads files: `singular` names the
    // written file, `kind` goes into a link and into `<CrdForm kind=…>`.
    if (!/^[A-Za-z][A-Za-z0-9]*$/.test(kind)) throw new Error(`${file}: invalid kind ${JSON.stringify(kind)}`);
    if (!/^[a-z0-9]([-a-z0-9]*[a-z0-9])?$/.test(singular)) {
      throw new Error(`${file}: invalid singular ${JSON.stringify(singular)}`);
    }
    const version = crd.spec.versions[0];
    const specSchema = version.schema.openAPIV3Schema.properties.spec;
    const apiVersion = `${crd.spec.group}/${version.name}`;
    const scope = crd.spec.scope;
    groups.add(crd.spec.group);

    const description = (specSchema.description ?? `${kind} spec.`).trim();
    const summary = shortDescription(description);
    const ctx = { kind, anchors: new Map(), undescribed: 0 };
    const fields = renderObject(specSchema, "spec", ctx);
    const sections = renderSections(specSchema, "spec", ctx);
    if (ctx.undescribed) console.warn(`${kind}: ${ctx.undescribed} field${ctx.undescribed === 1 ? "" : "s"} without a description`);
    const shortNames = crd.spec.names.shortNames ?? [];
    const [guideTitle, guideHref] = guides[kind] ?? [];

    checkExample(kind, examples[kind], specSchema);
    const metadata = { name: `my-${kind.replace(prefix, "").toLowerCase()}` };
    if (scope === "Namespaced") metadata.namespace = exampleNamespace;
    const example = yaml
      .dump({ apiVersion, kind, metadata, spec: examples[kind] }, { lineWidth: 100 })
      .trim();

    const facts = [
      `- **API version:** ${codeSpan(apiVersion)}`,
      `- **Scope:** ${scopeLabel(scope)}`,
      `- **Short names:** ${codeList(shortNames) || "none"}`,
      ...(guideTitle ? [`- **Guide:** [${guideTitle}](${guideHref})`] : []),
    ].join("\n");

    const frontmatter = yaml
      // `full`: the page widens to fit the manifest builder's form and preview
      // side by side (createDocsPage keeps its TOC); prose keeps its 68ch
      // measure regardless.
      // Backticks and bold are stripped: the description renders as plain text.
      .dump({ title: kind, description: summary.replace(/`|\*\*/g, ""), full: true })
      .trim();
    const mdx = `---\n${frontmatter}\n---\n\n${facts}\n\n${mdxText(description)}\n\n## Example\n\n\`\`\`yaml\n${example}\n\`\`\`\n\n## Spec fields\n\n${[fields, ...sections].join("\n\n")}\n\n## Build a manifest\n\nAnswer the questions and fill in the fields: the YAML preview updates as you type, ready to copy or download and \`kubectl apply\`.\n\n<CrdForm kind="${kind}" />\n`;
    writes.push([path.join(mdxOutDir, `${singular}.mdx`), mdx]);

    // apiVersion/kind/scope aren't JSON Schema keywords — added here purely
    // for the client-side form (CrdForm) to assemble a full manifest
    // (apiVersion + kind + metadata.namespace-or-not) without a second
    // fetch. Consumers that want strict JSON Schema should ignore them.
    const cleanSchema = { apiVersion, kind, scope, ...stripK8sExtensions(specSchema) };
    writes.push([path.join(schemaOutDir, `${singular}.schema.json`), `${JSON.stringify(cleanSchema, null, 2)}\n`]);

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
      `[${codeSpan(k.kind, { cell: true })}](${baseUrl}/${k.singular})`,
      ...(withShortNames ? [codeList(k.shortNames, { cell: true })] : []),
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
  writes.push([path.join(mdxOutDir, "index.mdx"), indexMdx]);
  writes.push([
    path.join(mdxOutDir, "meta.json"),
    `${JSON.stringify({ title: "CRD reference", pages: ["index", ...index.map((k) => k.singular)] }, null, 2)}\n`,
  ]);
  // The kind picker of `<CrdBuilder />` reads this: every CRD, without a
  // second data source to keep in sync.
  writes.push([
    path.join(schemaOutDir, "index.json"),
    `${JSON.stringify(index.map(({ kind, singular, scope }) => ({ kind, singular, scope })), null, 2)}\n`,
  ]);

  fs.mkdirSync(mdxOutDir, { recursive: true });
  fs.mkdirSync(schemaOutDir, { recursive: true });
  clean(mdxOutDir, [".mdx", "meta.json"]);
  clean(schemaOutDir, [".json"]);
  for (const [file, content] of writes) fs.writeFileSync(file, content);

  if (config.reference) {
    if (groups.size !== 1) throw new Error(`the CRDs span ${groups.size} API groups, the reference needs one`);
    generateReference(config.reference, { root, group: [...groups][0] });
  }

  return files.length;
}
