// The project is its YAML: multi-document in, multi-document (or one file
// per resource) out. A recipe is imported the same way.

import * as yaml from "js-yaml";
import { pruneValue } from "../../crd-form/schema-utils";
import { isRecord } from "../../crd-form/types";
import { gkKey, parseApiVersion } from "./catalog";
import { buildGraph, dependencies } from "./graph";
import { addResource, groupKindOf, isNamespaced, nameOf, namespaceOf } from "./project";
import type { Catalog, Doc, Issue, Project, Resource, ResourceMode } from "./types";

/**
 * Appends every document of `text` to `project`. A document that is not a
 * named Kubernetes object is refused (an issue, nothing added); one of a
 * kind outside the catalog, or in another version, is kept read-only.
 */
export function importYaml(project: Project, catalog: Catalog, text: string): { project: Project; issues: Issue[] } {
  let docs: unknown[];
  try {
    docs = yaml.loadAll(text);
  } catch (error) {
    return { project, issues: [{ severity: "error", code: "invalid-document", message: `not YAML: ${String(error)}` }] };
  }
  const issues: Issue[] = [];
  let out = project;
  docs.forEach((doc, index) => {
    if (doc === null || doc === undefined) return;
    const where = `document ${index + 1}`;
    if (!isRecord(doc) || typeof doc.apiVersion !== "string" || typeof doc.kind !== "string" || !nameOf(doc)) {
      issues.push({ severity: "error", code: "invalid-document", message: `${where}: not a named object (apiVersion, kind, metadata.name)` });
      return;
    }
    const { mode, issue } = modeOf(catalog, doc);
    const added = addResource(out, doc, mode);
    out = added.project;
    if (issue) issues.push({ ...issue, resource: added.id });
  });
  return { project: out, issues };
}

function modeOf(catalog: Catalog, doc: Doc): { mode: ResourceMode; issue?: Omit<Issue, "resource"> } {
  const gk = groupKindOf(doc);
  const entry = gk && catalog.get(gkKey(gk));
  const label = `${String(doc.kind)} ${nameOf(doc)}`;
  if (!entry) return { mode: "unknown", issue: { severity: "info", code: "unknown-kind", message: `${label}: ${String(doc.apiVersion)} is not in the catalog, kept as is` } };
  const version = typeof doc.apiVersion === "string" ? parseApiVersion(doc.apiVersion)?.version : undefined;
  if (version === entry.version) return { mode: "editable" };
  if (version !== undefined && entry.served.includes(version)) {
    return { mode: "readOnly", issue: { severity: "warning", code: "other-version", message: `${label}: ${String(doc.apiVersion)} is served but not ${entry.apiVersion}, read-only` } };
  }
  return { mode: "readOnly", issue: { severity: "error", code: "unsupported-version", message: `${label}: ${String(doc.apiVersion)} is not a served version` } };
}

/** The resources, every one after what it depends on; otherwise in project order. */
export function dependencyOrder(project: Project, catalog: Catalog): Resource[] {
  const deps = dependencies(buildGraph(project, catalog));
  // A made object stands for its maker.
  const owner = (id: string) => id.split("/produces/")[0] ?? id;
  const ids = new Set(project.resources.map((r) => r.id));
  const placed = new Set<string>();
  const ordered: Resource[] = [];
  let pending = project.resources;
  while (pending.length) {
    const ready = pending.find((r) =>
      [...(deps.get(r.id) ?? [])].map(owner).every((d) => d === r.id || placed.has(d) || !ids.has(d)),
    );
    // A cycle: the first one left goes next.
    const next = ready ?? pending[0];
    if (!next) break;
    ordered.push(next);
    placed.add(next.id);
    pending = pending.filter((r) => r !== next);
  }
  return ordered;
}

/** The manifest as written: an editable one pruned of empty fields and given its namespace. */
export function exportDoc(project: Project, catalog: Catalog, resource: Resource): Doc {
  if (resource.mode !== "editable") return resource.doc;
  const gk = groupKindOf(resource.doc);
  const entry = gk && catalog.get(gkKey(gk));
  const metadata: Record<string, unknown> = { ...(isRecord(resource.doc.metadata) ? resource.doc.metadata : {}) };
  const namespace = namespaceOf(project, catalog, resource);
  if (namespace === undefined) delete metadata.namespace;
  else metadata.namespace = namespace;
  const doc: Doc = { ...resource.doc, metadata };
  // Until its schema is loaded, a spec is written as it is.
  if (entry?.schema) doc.spec = pruneValue(entry.schema, resource.doc.spec);
  return doc;
}

const dump = (doc: Doc) => yaml.dump(doc, { noRefs: true, lineWidth: 100 });

export function exportYaml(project: Project, catalog: Catalog): string {
  return dependencyOrder(project, catalog)
    .map((r) => dump(exportDoc(project, catalog, r)))
    .join("---\n");
}

/** One file per resource, numbered in dependency order: `01-secret-token.yaml`. */
export function exportFiles(project: Project, catalog: Catalog): { name: string; content: string }[] {
  const ordered = dependencyOrder(project, catalog);
  const width = String(ordered.length).length < 2 ? 2 : String(ordered.length).length;
  return ordered.map((r, i) => {
    const gk = groupKindOf(r.doc);
    const namespaced = gk ? isNamespaced(catalog, gk) : true;
    const ns = namespaced ? namespaceOf(project, catalog, r) : undefined;
    const slug = [String(r.doc.kind), ns, nameOf(r.doc)]
      .filter((part): part is string => Boolean(part))
      .join("-")
      .toLowerCase()
      .replace(/[^a-z0-9.-]+/g, "-");
    return { name: `${String(i + 1).padStart(width, "0")}-${slug}.yaml`, content: dump(exportDoc(project, catalog, r)) };
  });
}
