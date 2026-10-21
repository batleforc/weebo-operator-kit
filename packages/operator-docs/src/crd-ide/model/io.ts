// The project is its YAML: multi-document in, multi-document (or one file
// per resource) out. A recipe is imported the same way.

import * as yaml from "js-yaml";
import { pruneValue } from "../../crd-form/schema-utils";
import { isRecord } from "../../crd-form/types";
import { gkKey, parseApiVersion } from "./catalog";
import { buildGraph, dependencies } from "./graph";
import { addExternal, addResource, groupKindOf, isNamespaced, nameOf, namespaceOf } from "./project";
import type { Catalog, Doc, Issue, Project, Resource, ResourceMode } from "./types";

/** Server-set metadata: never part of a manifest one writes. */
const SERVER_METADATA = ["uid", "resourceVersion", "generation", "creationTimestamp", "deletionTimestamp", "deletionGracePeriodSeconds", "managedFields", "selfLink"];
const LAST_APPLIED = "kubectl.kubernetes.io/last-applied-configuration";

/** A manifest as one writes it: no `status`, no server-set metadata. */
export function cleanDoc(doc: Doc): Doc {
  const { status: _status, ...rest } = doc;
  if (!isRecord(rest.metadata)) return rest;
  const metadata: Record<string, unknown> = { ...rest.metadata };
  for (const key of SERVER_METADATA) delete metadata[key];
  if (isRecord(metadata.annotations)) {
    const { [LAST_APPLIED]: _applied, ...annotations } = metadata.annotations;
    if (Object.keys(annotations).length) metadata.annotations = annotations;
    else delete metadata.annotations;
  }
  return { ...rest, metadata };
}

/** The objects of a document: itself, or the items of a `List` (`kubectl get -o yaml`). */
const objectsOf = (doc: unknown): unknown[] =>
  isRecord(doc) && typeof doc.kind === "string" && doc.kind.endsWith("List") && Array.isArray(doc.items) ? doc.items : [doc];

const isSecret = (doc: Doc) => doc.apiVersion === "v1" && doc.kind === "Secret";

/**
 * Adds every object of `text` to `project`: multi-document YAML, `List`s
 * included, cleaned of what the server sets. One that has the identity
 * (kind, namespace, name) of a resource of the project replaces it. A
 * document that is not a named object is refused (an issue, nothing
 * added); one of a kind outside the catalog, or in another version, is
 * kept read-only. A `Secret` becomes an external object: its keys are
 * kept, never its values.
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
  docs.flatMap(objectsOf).forEach((raw, index) => {
    if (raw === null || raw === undefined) return;
    const where = `object ${index + 1}`;
    if (!isRecord(raw) || typeof raw.apiVersion !== "string" || typeof raw.kind !== "string" || !nameOf(raw)) {
      issues.push({ severity: "error", code: "invalid-document", message: `${where}: not a named object (apiVersion, kind, metadata.name)` });
      return;
    }
    const doc = cleanDoc(raw);
    if (isSecret(doc)) {
      out = importSecret(out, doc);
      return;
    }
    const { mode, issue } = modeOf(catalog, doc);
    const same = out.resources.find((r) => sameObject(out, catalog, r, doc));
    let id: string;
    if (same) {
      id = same.id;
      out = { ...out, resources: out.resources.map((r) => (r.id === same.id ? { ...r, doc, mode } : r)) };
    } else {
      const added = addResource(out, doc, mode);
      ({ project: out, id } = added);
    }
    if (issue) issues.push({ ...issue, resource: id });
  });
  return { project: out, issues };
}

/** Whether `doc` names the same object as `resource`: kind, namespace and name. */
function sameObject(project: Project, catalog: Catalog, resource: Resource, doc: Doc): boolean {
  const a = groupKindOf(resource.doc);
  const b = groupKindOf(doc);
  if (!a || !b || gkKey(a) !== gkKey(b) || nameOf(resource.doc) !== nameOf(doc)) return false;
  return namespaceOf(project, catalog, resource) === namespaceOf(project, catalog, { id: "", doc, mode: "editable" });
}

function importSecret(project: Project, doc: Doc): Project {
  const keys = [
    ...new Set([...Object.keys(isRecord(doc.data) ? doc.data : {}), ...Object.keys(isRecord(doc.stringData) ? doc.stringData : {})]),
  ].sort();
  const metadata = isRecord(doc.metadata) ? doc.metadata : {};
  const namespace = typeof metadata.namespace === "string" && metadata.namespace ? metadata.namespace : undefined;
  const name = nameOf(doc);
  const others = project.externals.filter(
    (x) => !(x.kind.group === "" && x.kind.kind === "Secret" && x.name === name && (x.namespace ?? project.defaultNamespace) === (namespace ?? project.defaultNamespace)),
  );
  return addExternal({ ...project, externals: others }, { kind: { group: "", kind: "Secret" }, name, ...(namespace ? { namespace } : {}), keys }).project;
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
