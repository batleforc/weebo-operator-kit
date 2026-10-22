// CRD reference generator: from the operator's CRD YAML (what its `crdgen`
// produces via `CustomResourceExt::crd()`) to
//   - <contentDir>/<singular>.mdx — facts, an example, the spec field table
//     with a section per nested object (RFC 0004), and the manifest builder
//     (`<CrdForm kind=… />`), one page per kind;
//   - <contentDir>/index.mdx + meta.json — the section's table of kinds and
//     its sidebar order;
//   - <schemaDir>/<singular>.schema.json — the spec schema the form fetches,
//     and <schemaDir>/index.json — the kind list `<CrdBuilder />` offers;
//   - <schemaDir>/links.json — which fields reference another object and
//     which objects a kind produces, for the CRD IDE: the crdgen's
//     <crdDir>/links.json, merged with the config's `links`;
//   - the schematic kinds of that links.json (kinds the IDE draws but
//     Kubernetes never sees): a schema each and an index.json entry marked
//     `schematic`, no page.
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
 * @property {boolean} [pages] `false`: only the schemas, index.json and
 *   links.json (a catalog of another tool's CRDs, for the CRD IDE): no page,
 *   no example needed. Default `true`.
 * @property {CrdLinks} [links] Reference fields and produced objects the
 *   crdgen's `links.json` does not declare (a CRD written by another tool,
 *   an object the kind makes). Every path is checked against the CRD.
 * @property {import("../reference/generate.mjs").ReferenceConfig} [reference]
 *   When set, also generates the operator reference pages (reason codes,
 *   Events, metrics & health, resource budget) from the operator's sources.
 */

/**
 * A Kubernetes kind; `group` absent or `""` is the core group.
 * @typedef {{ group?: string, kind: string }} GroupKind
 */

/**
 * A field naming another object (`spec.instanceRef`). Paths are dotted from
 * the object's root; `[]` steps into an array's items, `*` into a map's
 * values.
 * @typedef {{ path: string, to: GroupKind[] }} Reference
 */

/**
 * Objects a resource of this kind makes, such as the Secret an external
 * secret writes: their kind, one per value `each` reaches (default: one, the
 * resource itself), the fields that name it (the first one set wins,
 * `metadata.name` as a last resort), its namespace's field (default: the
 * resource's), and its keys: a field listing them (`keys`) or a fixed list
 * (`keyNames`). `unless` skips an item whose field holds a value. Paths
 * other than `each` are relative to each item. `template` names it when no
 * `name` field is set: `{path}` relative to the item, `{/path}` from the
 * resource's root, `{/ref>path}` read on the resource the reference field
 * `ref` designates.
 * @typedef {{ kind: GroupKind, each?: string, name: string[], template?: string, namespace?: string, keys?: string, keyNames?: string[], unless?: { path: string, equals: string } }} Production
 */

/**
 * Per kind, what `weebo_kit_api::links` cannot say: references of a CRD
 * written by another tool, and the objects the kind produces.
 * @typedef {Record<string, { references?: Reference[], produces?: Production[] }>} CrdLinks
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
// A `#` opening a line (rustdoc's `# Examples`) would become a heading;
// `^` only counts where the text itself starts a line.
const escapeOutsideCode = (text, lineStart) =>
  text
    .replace(PROSE_ESCAPES, "\\$&")
    .replace(lineStart ? /(^|\n)([ \t]*)#/g : /(\n)([ \t]*)#/g, "$1$2\\#");

/** Code spans left as written, the rest escaped; spans never cross `text`. */
function escapeParagraph(text) {
  let out = "";
  let rest = text;
  for (let open = rest.match(/`+/); open; open = rest.match(/`+/)) {
    const fence = open[0];
    const after = rest.slice(open.index + fence.length);
    const close = after.match(new RegExp(`(?<!\`)${fence}(?!\`)`));
    out += escapeOutsideCode(rest.slice(0, open.index), out === "");
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
  return out + escapeOutsideCode(rest, out === "");
}

/**
 * CRD prose, made safe to paste into MDX: outside code spans, `\ { } < >`,
 * the `!` of `![` and a `#` opening a line are escaped, so no description can
 * become a JS expression (evaluated at build time), a JSX element, an image
 * fetched at build time or a heading.
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

const METADATA_PATHS = new Set(["metadata.name", "metadata.namespace"]);

// A free-form object: any path under it may hold a value.
const isOpen = (node) => node["x-kubernetes-preserve-unknown-fields"] === true || (node.type === "object" && !node.properties && node.additionalProperties === undefined);

// Whether `fieldPath` (`spec.a[].b`, `spec.m.*`) is a field of `schema`, the
// CRD's openAPIV3Schema. `metadata` is not in a generated schema: its name
// and namespace are always there. Under a free-form object, any path is.
function hasPath(schema, fieldPath) {
  if (METADATA_PATHS.has(fieldPath)) return true;
  let nodes = [schema];
  for (const segment of fieldPath.split(".")) {
    if (nodes.flatMap(variants).some(isOpen)) return true;
    const [, name, arrays] = /^([^[\]]+)((?:\[\])*)$/.exec(segment) ?? [];
    if (!name) return false;
    nodes = nodes
      .flatMap(variants)
      .map((n) => (name === "*" ? (typeof n.additionalProperties === "object" ? n.additionalProperties : undefined) : n.properties?.[name]))
      .filter(Boolean);
    for (let i = 0; i < arrays.length / 2; i++) nodes = nodes.flatMap(variants).map((n) => n.items).filter(Boolean);
    if (!nodes.length) return false;
  }
  return true;
}

// A node and the variants of its `allOf`/`anyOf`/`oneOf`, recursively.
const variants = (node) => [node, ...["allOf", "anyOf", "oneOf"].flatMap((k) => (node[k] ?? []).flatMap(variants))];

function checkGroupKind(where, gk) {
  if (typeof gk?.kind !== "string" || !/^[A-Za-z][A-Za-z0-9]*$/.test(gk.kind) || (gk.group !== undefined && typeof gk.group !== "string")) {
    throw new Error(`${where}: invalid kind ${JSON.stringify(gk)}`);
  }
  return { group: gk.group ?? "", kind: gk.kind };
}

/**
 * The published links: the crdgen's `links.json` (`{ version: 1, kinds:
 * { <Kind>: { references, produces } } }`, absent for an operator whose
 * crdgen predates it), plus the config's `links`. Every kind must be one of
 * the CRDs, every path one of its fields, and a reference is declared once.
 *
 * @param {Map<string, object>} schemas openAPIV3Schema per kind
 * @param {unknown} generated the parsed crdgen file, or undefined
 * @param {CrdLinks} configured
 */
export function buildLinks(schemas, generated, configured) {
  const kinds = {};
  const entry = (where, kind) => {
    if (!schemas.has(kind)) throw new Error(`${where}: ${kind} is not one of the CRDs`);
    return (kinds[kind] ??= { references: [], produces: [] });
  };
  const checkPath = (where, kind, fieldPath) => {
    if (typeof fieldPath !== "string" || !hasPath(schemas.get(kind), fieldPath)) {
      throw new Error(`${where}: ${kind} has no field ${JSON.stringify(fieldPath)}`);
    }
  };
  const addReference = (where, kind, ref) => {
    const links = entry(where, kind);
    checkPath(where, kind, ref?.path);
    if (links.references.some((r) => r.path === ref.path)) throw new Error(`${where}: ${kind} ${ref.path} is already a reference`);
    if (!Array.isArray(ref.to) || !ref.to.length) throw new Error(`${where}: ${kind} ${ref.path} references no kind`);
    links.references.push({ path: ref.path, to: ref.to.map((gk) => checkGroupKind(`${where}: ${kind} ${ref.path}`, gk)) });
  };
  // Every placeholder a field: of the item, of the resource, or of a kind
  // the reference (one of the resource's) designates.
  const checkTemplate = (where, kind, p, within) => {
    const holes = typeof p.template === "string" ? [...p.template.matchAll(/\{([^{}]*)\}/g)].map((m) => m[1]) : [];
    if (!holes.length) throw new Error(`${where}: ${kind} template ${JSON.stringify(p.template)} has no {field}`);
    for (const hole of holes) {
      if (!hole.startsWith("/")) {
        checkPath(where, kind, within(hole));
        continue;
      }
      const [ref, field, ...rest] = hole.slice(1).split(">");
      if (field === undefined) {
        checkPath(where, kind, ref);
        continue;
      }
      const link = kinds[kind]?.references.find((r) => r.path === ref);
      if (rest.length || !link) throw new Error(`${where}: ${kind} template {${hole}}: ${ref} is not one of its references`);
      const targets = link.to.map((gk) => schemas.get(gk.kind)).filter(Boolean);
      if (targets.length && !targets.some((schema) => hasPath(schema, field))) {
        throw new Error(`${where}: ${kind} template {${hole}}: no ${link.to.map((gk) => gk.kind).join(" or ")} has a field ${field}`);
      }
    }
  };
  const addProduction = (where, kind, p) => {
    const links = entry(where, kind);
    if (!Array.isArray(p?.name) || (!p.name.length && p.template === undefined)) throw new Error(`${where}: ${kind} produces an object with no name field`);
    if (p.each !== undefined) checkPath(where, kind, p.each);
    // `.`: the item itself (a plain string), only with `each`.
    if (p.name.includes(".") && p.each === undefined) throw new Error(`${where}: ${kind} names an object by "." without each`);
    const within = (rel) => (rel === "." ? p.each : p.each ? `${p.each}.${rel}` : rel);
    const relative = [...p.name, p.namespace, p.keys, p.unless?.path].filter((v) => v !== undefined);
    for (const rel of relative) checkPath(where, kind, typeof rel === "string" ? within(rel) : rel);
    if (p.keyNames !== undefined && !(Array.isArray(p.keyNames) && p.keyNames.every((k) => typeof k === "string"))) {
      throw new Error(`${where}: ${kind} keyNames is not a list of names`);
    }
    if (p.unless !== undefined && typeof p.unless.equals !== "string") throw new Error(`${where}: ${kind} unless.equals is not a string`);
    if (p.template !== undefined) checkTemplate(where, kind, p, within);
    links.produces.push({
      kind: checkGroupKind(`${where}: ${kind}`, p.kind),
      ...(p.each !== undefined ? { each: p.each } : {}),
      name: p.name,
      ...(p.template !== undefined ? { template: p.template } : {}),
      ...(p.namespace !== undefined ? { namespace: p.namespace } : {}),
      ...(p.keys !== undefined ? { keys: p.keys } : {}),
      ...(p.keyNames?.length ? { keyNames: p.keyNames } : {}),
      ...(p.unless !== undefined ? { unless: { path: p.unless.path, equals: p.unless.equals } } : {}),
    });
  };

  if (generated !== undefined) {
    const where = "links.json";
    if (generated?.version !== 1 || typeof generated.kinds !== "object") {
      throw new Error(`${where}: expected { version: 1, kinds }, rerun the crdgen`);
    }
    for (const [kind, { references = [], produces = [] }] of Object.entries(generated.kinds)) {
      entry(`${where} (stale? rerun the crdgen)`, kind);
      for (const ref of references) addReference(where, kind, ref);
      for (const p of produces) addProduction(where, kind, p);
    }
    for (const [kind, { references = [], produces = [] }] of Object.entries(generated.schematic ?? {})) {
      entry(`${where} schematic`, kind);
      for (const ref of references) addReference(`${where} schematic`, kind, ref);
      for (const p of produces) addProduction(`${where} schematic`, kind, p);
    }
  }
  for (const [kind, { references = [], produces = [] }] of Object.entries(configured)) {
    const where = "config links";
    for (const ref of references) addReference(where, kind, ref);
    for (const p of produces) addProduction(where, kind, p);
  }
  for (const links of Object.values(kinds)) links.references.sort((a, b) => a.path.localeCompare(b.path));
  return { version: 1, kinds: Object.fromEntries(Object.entries(kinds).sort(([a], [b]) => a.localeCompare(b))) };
}

/**
 * The schematic kinds of the crdgen's `links.json` (`schematic: { <Kind>:
 * { group, scope, schema, references, produces } }`): kinds the CRD IDE
 * draws and links but never exports with the manifests, all in version
 * `v1` of their group. A name already a CRD's is refused.
 *
 * @param {unknown} generated the parsed crdgen file, or undefined
 * @param {Set<string>} crdKinds
 * @returns {{ kind: string, singular: string, group: string, scope: "Namespaced" | "Cluster", schema: Record<string, unknown> }[]}
 */
export function readSchematic(generated, crdKinds) {
  const kinds = generated?.schematic ?? {};
  if (typeof kinds !== "object" || Array.isArray(kinds)) throw new Error("links.json: schematic is not a map of kinds");
  return Object.entries(kinds)
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([kind, entry]) => {
      const where = `links.json schematic ${kind}`;
      const { group } = checkGroupKind(where, { group: entry?.group, kind });
      if (!/^[a-z0-9]([-a-z0-9]*[a-z0-9])?(\.[a-z0-9]([-a-z0-9]*[a-z0-9])?)+$/.test(group)) throw new Error(`${where}: invalid group ${JSON.stringify(group)}`);
      if (entry.scope !== "Namespaced" && entry.scope !== "Cluster") throw new Error(`${where}: scope is Namespaced or Cluster`);
      if (typeof entry.schema !== "object" || entry.schema === null) throw new Error(`${where}: no schema`);
      if (crdKinds.has(kind)) throw new Error(`${where}: already a CRD's kind`);
      return { kind, singular: kind.toLowerCase(), group, scope: entry.scope, schema: entry.schema };
    });
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
  const pages = config.pages ?? true;

  // Written only once every page is built: a refused CRD leaves the previous
  // pages in place, not half of a new set.
  const writes = [];

  const group = config.reference?.group;
  // A component without CRDs (only config and admission policies) may have
  // no CRD directory at all, as long as it names its group for the reference.
  const files = (group && !fs.existsSync(crdDir) ? [] : fs.readdirSync(crdDir))
    .filter((f) => f.endsWith(".yaml"))
    .sort();
  const index = [];
  const groups = new Set();
  const schemas = new Map();

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
    // The storage version: the one the IDE and the form write. The other
    // served ones are listed, so a manifest written in one of them is
    // recognised.
    const version = crd.spec.versions.find((v) => v.storage) ?? crd.spec.versions[0];
    const served = crd.spec.versions.filter((v) => v.served !== false).map((v) => v.name);
    const specSchema = version.schema.openAPIV3Schema.properties.spec;
    schemas.set(kind, version.schema.openAPIV3Schema);
    const apiVersion = `${crd.spec.group}/${version.name}`;
    const scope = crd.spec.scope;
    groups.add(crd.spec.group);

    const description = (specSchema.description ?? `${kind} spec.`).trim();
    const summary = shortDescription(description);
    const shortNames = crd.spec.names.shortNames ?? [];
    index.push({ kind, singular, scope, shortNames, summary, apiVersion, served });

    // apiVersion/kind/scope/served aren't JSON Schema keywords — added here
    // purely for the client-side form (CrdForm) and IDE to assemble a full
    // manifest (apiVersion + kind + metadata.namespace-or-not) without a
    // second fetch. Consumers that want strict JSON Schema should ignore them.
    const cleanSchema = { apiVersion, kind, scope, served, ...stripK8sExtensions(specSchema) };
    writes.push([path.join(schemaOutDir, `${singular}.schema.json`), `${JSON.stringify(cleanSchema, null, 2)}\n`]);
    if (!pages) continue;

    const ctx = { kind, anchors: new Map(), undescribed: 0 };
    const fields = renderObject(specSchema, "spec", ctx);
    const sections = renderSections(specSchema, "spec", ctx);
    if (ctx.undescribed) console.warn(`${kind}: ${ctx.undescribed} field${ctx.undescribed === 1 ? "" : "s"} without a description`);
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
  if (pages) {
    writes.push([path.join(mdxOutDir, "index.mdx"), indexMdx]);
    writes.push([
      path.join(mdxOutDir, "meta.json"),
      `${JSON.stringify({ title: "CRD reference", pages: ["index", ...index.map((k) => k.singular)] }, null, 2)}\n`,
    ]);
  }
  const linksFile = path.join(crdDir, "links.json");
  const generatedLinks = fs.existsSync(linksFile) ? JSON.parse(fs.readFileSync(linksFile, "utf8")) : undefined;
  const schematic = readSchematic(generatedLinks, new Set(schemas.keys()));
  for (const s of schematic) {
    if (index.some((k) => k.singular === s.singular)) throw new Error(`links.json schematic ${s.kind}: ${s.singular} is already a CRD's file`);
    // Its links' paths start at the object's root, as a CRD's.
    schemas.set(s.kind, { type: "object", properties: { spec: s.schema } });
    const apiVersion = `${s.group}/v1`;
    writes.push([
      path.join(schemaOutDir, `${s.singular}.schema.json`),
      `${JSON.stringify({ apiVersion, kind: s.kind, scope: s.scope, served: ["v1"], schematic: true, ...s.schema }, null, 2)}\n`,
    ]);
  }

  // The kind picker of `<CrdBuilder />` and the IDE's catalog read this:
  // every CRD, without a second data source to keep in sync, then the
  // schematic kinds (the builder leaves them out). The IDE loads a kind's
  // schema only once a resource of that kind needs it.
  writes.push([
    path.join(schemaOutDir, "index.json"),
    `${JSON.stringify(
      [
        ...index.map(({ kind, singular, scope, apiVersion, served }) => ({ kind, singular, scope, apiVersion, served })),
        ...schematic.map(({ kind, singular, scope, group }) => ({ kind, singular, scope, apiVersion: `${group}/v1`, served: ["v1"], schematic: true })),
      ],
      null,
      2,
    )}\n`,
  ]);

  const links = buildLinks(schemas, generatedLinks, config.links ?? {});
  writes.push([path.join(schemaOutDir, "links.json"), `${JSON.stringify(links, null, 2)}\n`]);

  if (config.reference) {
    if (groups.size > 1) throw new Error(`the CRDs span ${groups.size} API groups, the reference needs one`);
    const [crdGroup] = groups;
    if (!crdGroup && !group) {
      throw new Error(`no CRD in ${crdDir}: set reference.group, the API group the reference pages name`);
    }
    if (crdGroup && group && crdGroup !== group) {
      throw new Error(`reference.group is ${group}, but the CRDs' group is ${crdGroup}`);
    }
  }

  // With no CRD and a reference group, the component has no CRD section:
  // nothing is written there.
  if (files.length || !group) {
    fs.mkdirSync(schemaOutDir, { recursive: true });
    if (pages) {
      fs.mkdirSync(mdxOutDir, { recursive: true });
      clean(mdxOutDir, [".mdx", "meta.json"]);
    }
    clean(schemaOutDir, [".json"]);
    for (const [file, content] of writes) fs.writeFileSync(file, content);
  }

  if (config.reference) generateReference(config.reference, { root, group: group ?? [...groups][0] });

  return files.length;
}
