// A project's resources: identity, namespace, and the edits the views make.
// Every edit returns a new project; none mutates its input.

import { isRecord } from "../../crd-form/types";
import { gkKey, parseApiVersion } from "./catalog";
import type { Catalog, Doc, External, GroupKind, Project, Resource, ResourceMode } from "./types";

export const emptyProject = (defaultNamespace = "default"): Project => ({
  defaultNamespace,
  resources: [],
  externals: [],
});

export function groupKindOf(doc: Doc): GroupKind | undefined {
  const parsed = typeof doc.apiVersion === "string" ? parseApiVersion(doc.apiVersion) : undefined;
  return parsed && typeof doc.kind === "string" ? { group: parsed.group, kind: doc.kind } : undefined;
}

const metadataOf = (doc: Doc): Record<string, unknown> => (isRecord(doc.metadata) ? doc.metadata : {});

export function nameOf(doc: Doc): string {
  const name = metadataOf(doc).name;
  return typeof name === "string" ? name : "";
}

/**
 * Whether a kind is namespaced: the catalog's scope, else namespaced (every
 * core kind a reference names — Secret, ConfigMap — is).
 */
export function isNamespaced(catalog: Catalog, gk: GroupKind): boolean {
  return catalog.get(gkKey(gk))?.scope !== "Cluster";
}

/** The resource's namespace: its own, else the project's; none for a cluster-scoped kind. */
export function namespaceOf(project: Project, catalog: Catalog, resource: Resource): string | undefined {
  const gk = groupKindOf(resource.doc);
  if (gk && !isNamespaced(catalog, gk)) return undefined;
  const own = metadataOf(resource.doc).namespace;
  return typeof own === "string" && own ? own : project.defaultNamespace;
}

function nextId(project: Project, prefix: string): string {
  const used = new Set([...project.resources, ...project.externals].map((r) => r.id));
  let n = used.size + 1;
  while (used.has(`${prefix}${n}`)) n++;
  return `${prefix}${n}`;
}

export function addResource(project: Project, doc: Doc, mode: ResourceMode = "editable"): { project: Project; id: string } {
  const id = nextId(project, "r");
  return { project: { ...project, resources: [...project.resources, { id, doc, mode }] }, id };
}

/** A blank resource of a catalog kind, in its storage version. */
export function newDoc(catalog: Catalog, gk: GroupKind, name: string): Doc {
  const entry = catalog.get(gkKey(gk));
  if (!entry) throw new Error(`${gkKey(gk)} is not in the catalog`);
  return { apiVersion: entry.apiVersion, kind: gk.kind, metadata: { name }, spec: {} };
}

/** Replaces an editable resource's document; a read-only or unknown one stays as imported. */
export function updateResource(project: Project, id: string, doc: Doc): Project {
  return {
    ...project,
    resources: project.resources.map((r) => (r.id === id && r.mode === "editable" ? { ...r, doc } : r)),
  };
}

export function removeResource(project: Project, id: string): Project {
  return { ...project, resources: project.resources.filter((r) => r.id !== id) };
}

export function addExternal(project: Project, external: Omit<External, "id">): { project: Project; id: string } {
  const id = nextId(project, "x");
  return { project: { ...project, externals: [...project.externals, { ...external, id }] }, id };
}

export function removeExternal(project: Project, id: string): Project {
  return { ...project, externals: project.externals.filter((x) => x.id !== id) };
}

export const setDefaultNamespace = (project: Project, defaultNamespace: string): Project => ({
  ...project,
  defaultNamespace,
});
