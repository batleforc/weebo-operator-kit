// What a project's manifests say about each other: the objects they make,
// and which object each reference field designates. Derived on every call,
// never stored: renaming a resource re-points what names it.

import { isRecord } from "../../crd-form/types";
import { gkKey } from "./catalog";
import { expand, getAt, setAt } from "./paths";
import { groupKindOf, isNamespaced, nameOf, namespaceOf } from "./project";
import type { Catalog, Doc, GroupKind, Project, Reference, Segment } from "./types";

export interface GraphNode {
  /** The resource's or the external's id; `<resource id>/produces/<n>` for a made object. */
  id: string;
  kind: GroupKind;
  name: string;
  namespace: string | undefined;
  origin: "resource" | "produced" | "external";
  /** The keys it holds, when known (a produced Secret's `keys` field, an external's list). */
  keys?: string[];
  /** The resource that makes a produced object. */
  from?: string;
}

/** What a reference field holds: a bare name, or `{ name, namespace?, key? }`. */
export interface RefValue {
  name: string;
  namespace?: string;
  key?: string;
}

/** One reference field of one resource, concrete (`spec.keys[2]`). */
export interface Slot {
  resource: string;
  link: Reference;
  path: Segment[];
  ref: RefValue | undefined;
  /** The namespace the target is looked up in; `undefined` for a cluster-scoped target or a missing one. */
  namespace: string | undefined;
  target: GraphNode | undefined;
}

export interface Graph {
  nodes: GraphNode[];
  slots: Slot[];
}

export function readRef(value: unknown): RefValue | undefined {
  if (typeof value === "string") return value ? { name: value } : undefined;
  if (!isRecord(value) || typeof value.name !== "string" || !value.name) return undefined;
  const ref: RefValue = { name: value.name };
  if (typeof value.namespace === "string" && value.namespace) ref.namespace = value.namespace;
  if (typeof value.key === "string" && value.key) ref.key = value.key;
  return ref;
}

const sameKind = (a: GroupKind, b: GroupKind) => a.group === b.group && a.kind === b.kind;

const isName = (v: unknown): v is string => typeof v === "string" && v !== "";

/** The first value a (relative) path reaches. */
const first = (item: unknown, path: string): unknown => expand(item, path)[0]?.value;

function producedNodes(project: Project, catalog: Catalog): GraphNode[] {
  return project.resources.flatMap((resource) => {
    const gk = groupKindOf(resource.doc);
    const produces = gk ? (catalog.get(gkKey(gk))?.links.produces ?? []) : [];
    const own = namespaceOf(project, catalog, resource);
    return produces.flatMap((production, n) => {
      const items = production.each ? expand(resource.doc, production.each) : [{ path: [], value: resource.doc }];
      return items.flatMap(({ value: item }, i): GraphNode[] => {
        const { unless } = production;
        if (unless && first(item, unless.path) === unless.equals) return [];
        const name = production.name.map((path) => first(item, path)).find(isName);
        if (!name) return [];
        const namespace = production.namespace ? first(item, production.namespace) : undefined;
        const node: GraphNode = {
          id: `${resource.id}/produces/${n}/${i}`,
          kind: production.kind,
          name,
          namespace: isNamespaced(catalog, production.kind) ? (isName(namespace) ? namespace : own) : undefined,
          origin: "produced",
          from: resource.id,
        };
        if (production.keyNames) node.keys = production.keyNames;
        else if (production.keys) node.keys = expand(item, production.keys).map(({ value }) => value).filter(isName);
        return [node];
      });
    });
  });
}

export function buildGraph(project: Project, catalog: Catalog): Graph {
  const nodes: GraphNode[] = [
    ...project.resources.flatMap((resource): GraphNode[] => {
      const kind = groupKindOf(resource.doc);
      return kind
        ? [{ id: resource.id, kind, name: nameOf(resource.doc), namespace: namespaceOf(project, catalog, resource), origin: "resource" }]
        : [];
    }),
    ...producedNodes(project, catalog),
    ...project.externals.map(
      (x): GraphNode => ({
        id: x.id,
        kind: x.kind,
        name: x.name,
        namespace: isNamespaced(catalog, x.kind) ? (x.namespace ?? project.defaultNamespace) : undefined,
        origin: "external",
        keys: x.keys,
      }),
    ),
  ];

  const slots = project.resources.flatMap((resource) => {
    const gk = groupKindOf(resource.doc);
    const references = gk ? (catalog.get(gkKey(gk))?.links.references ?? []) : [];
    const own = namespaceOf(project, catalog, resource);
    return references.flatMap((link) =>
      expand(resource.doc, link.path).map(({ path, value }): Slot => {
        const ref = readRef(value);
        // A namespaced target: the reference's namespace, else the
        // referencing object's (none for a cluster-scoped one).
        const namespaced = link.to.some((to) => isNamespaced(catalog, to));
        const namespace = namespaced ? (ref?.namespace ?? own) : undefined;
        const target = ref
          ? nodes.find(
              (n) =>
                n.name === ref.name &&
                link.to.some((to) => sameKind(to, n.kind)) &&
                (n.namespace === undefined || n.namespace === namespace),
            )
          : undefined;
        return { resource: resource.id, link, path, ref, namespace, target };
      }),
    );
  });

  return { nodes, slots };
}

/** The nodes a slot may point at: the right kind, any namespace (the view filters). */
export const candidates = (graph: Graph, slot: Slot): GraphNode[] =>
  graph.nodes.filter((n) => slot.link.to.some((to) => sameKind(to, n.kind)));

/** `resource → what it depends on`: its resolved references and, for a made object, its maker. */
export function dependencies(graph: Graph): Map<string, Set<string>> {
  const deps = new Map<string, Set<string>>(graph.nodes.map((n) => [n.id, new Set<string>()]));
  for (const node of graph.nodes) if (node.from) deps.get(node.id)?.add(node.from);
  for (const slot of graph.slots) if (slot.target && slot.target.id !== slot.resource) deps.get(slot.resource)?.add(slot.target.id);
  return deps;
}

/**
 * The reference fields of a resource with nothing in them yet, that a view
 * can offer to fill: only plain paths (no `[]` or `*`), whose item or key
 * is the view's to create.
 */
export function emptySlots(graph: Graph, project: Project, catalog: Catalog, resourceId: string): Slot[] {
  const resource = project.resources.find((r) => r.id === resourceId);
  const gk = resource && groupKindOf(resource.doc);
  if (!resource || !gk) return [];
  const own = namespaceOf(project, catalog, resource);
  const taken = new Set(graph.slots.filter((s) => s.resource === resourceId).map((s) => s.link.path));
  return (catalog.get(gkKey(gk))?.links.references ?? [])
    .filter((link) => !taken.has(link.path) && !/\[\]|(^|\.)\*(\.|$)/.test(link.path))
    .map((link) => ({
      resource: resourceId,
      link,
      path: link.path.split("."),
      ref: undefined,
      namespace: link.to.some((to) => isNamespaced(catalog, to)) ? own : undefined,
      target: undefined,
    }));
}

/**
 * The resource's document with `slot` pointing at `target` (or emptied):
 * a bare-name field gets the name; an object keeps its other fields
 * (`key`), takes the name, and names the namespace only when it differs
 * from the resource's own (always, from a cluster-scoped resource).
 */
export function pointAt(project: Project, catalog: Catalog, slot: Slot, target: GraphNode | undefined): Doc | undefined {
  const resource = project.resources.find((r) => r.id === slot.resource);
  if (!resource) return undefined;
  const current = getAt(resource.doc, slot.path);
  if (!target) return asDoc(setAt(resource.doc, slot.path, undefined));
  if (typeof current === "string") return asDoc(setAt(resource.doc, slot.path, target.name));
  const own = namespaceOf(project, catalog, resource);
  const next: Record<string, unknown> = { ...(isRecord(current) ? current : {}), name: target.name };
  if (target.namespace !== undefined && target.namespace !== own) next.namespace = target.namespace;
  else delete next.namespace;
  return asDoc(setAt(resource.doc, slot.path, next));
}

const asDoc = (v: unknown): Doc => (isRecord(v) ? v : {});
