// A project's issues: each manifest against its schema, each reference
// against the project. The CEL rules are left to the API server.

import { findMissingRequired } from "../../crd-form/schema-utils";
import { isRecord, type FieldSchema } from "../../crd-form/types";
import { gkKey } from "./catalog";
import { buildGraph, type Graph, type GraphNode, type Slot } from "./graph";
import { formatPath } from "./paths";
import { groupKindOf, isNamespaced, nameOf, namespaceOf } from "./project";
import type { Catalog, Issue, Project, Resource } from "./types";

/** A recipe's blank: a string value such as `<fill in>`. */
export const isHole = (v: unknown): boolean => typeof v === "string" && /^<[^<>]+>$/.test(v);

// RFC 1123 subdomain: what most kinds accept as `metadata.name`.
const NAME = /^[a-z0-9]([-a-z0-9]*[a-z0-9])?(\.[a-z0-9]([-a-z0-9]*[a-z0-9])?)*$/;

function typeMatches(schema: FieldSchema, value: unknown): boolean {
  switch (schema.type) {
    case "string":
      return typeof value === "string";
    case "boolean":
      return typeof value === "boolean";
    case "integer":
      return Number.isInteger(value);
    case "number":
      return typeof value === "number";
    case "object":
      return isRecord(value);
    case "array":
      return Array.isArray(value);
    case undefined:
      return true;
    default:
      // A type the published schema should not hold: not checked.
      return true;
  }
}

/** Type, enum, pattern, bounds and unknown fields; the required ones come from `findMissingRequired`. */
export function checkValue(schema: FieldSchema, value: unknown, path: string): Omit<Issue, "resource">[] {
  const at = (code: Issue["code"], message: string): Omit<Issue, "resource"> => ({ severity: "error", code, message: `${path}: ${message}`, path });
  if (value === null && schema.nullable) return [];
  if (isHole(value)) return [{ severity: "error", code: "hole", message: `${path}: to fill in (${String(value)})`, path }];
  if (!typeMatches(schema, value)) return [at("type", `expected ${schema.type ?? "a value"}`)];

  const issues: Omit<Issue, "resource">[] = [];
  if (schema.enum && !schema.enum.includes(String(value))) issues.push(at("enum", `one of ${schema.enum.join(", ")}`));
  if (typeof value === "string") {
    if (schema.pattern !== undefined && !matches(schema.pattern, value)) issues.push(at("pattern", `must match ${schema.pattern}`));
    const length = [...value].length;
    if (schema.minLength !== undefined && length < schema.minLength) issues.push(at("range", `at least ${schema.minLength} characters`));
    if (schema.maxLength !== undefined && length > schema.maxLength) issues.push(at("range", `at most ${schema.maxLength} characters`));
  }
  if (typeof value === "number") {
    if (schema.minimum !== undefined && value < schema.minimum) issues.push(at("range", `at least ${schema.minimum}`));
    if (schema.maximum !== undefined && value > schema.maximum) issues.push(at("range", `at most ${schema.maximum}`));
  }
  if (Array.isArray(value)) {
    if (schema.minItems !== undefined && value.length < schema.minItems) issues.push(at("range", `at least ${schema.minItems} items`));
    if (schema.maxItems !== undefined && value.length > schema.maxItems) issues.push(at("range", `at most ${schema.maxItems} items`));
    const items = schema.items;
    if (items) value.forEach((item: unknown, i) => issues.push(...checkValue(items, item, `${path}[${i}]`)));
  }
  if (isRecord(value)) {
    const values = schema.additionalProperties;
    for (const [key, child] of Object.entries(value)) {
      const childSchema = schema.properties?.[key] ?? (typeof values === "object" ? values : undefined);
      if (childSchema) issues.push(...checkValue(childSchema, child, `${path}.${key}`));
      // No `properties`: a free-form object (`x-kubernetes-preserve-unknown-fields`).
      else if (schema.properties && values === undefined) {
        issues.push({ severity: "error", code: "unknown-field", message: `${path}.${key}: unknown field`, path: `${path}.${key}` });
      }
    }
  }
  return issues;
}

function matches(pattern: string, value: string): boolean {
  try {
    return new RegExp(pattern, "u").test(value);
  } catch {
    // A pattern JavaScript cannot read (RE2-only syntax): the API server will check it.
    return true;
  }
}

function checkResource(project: Project, catalog: Catalog, resource: Resource): Issue[] {
  const issues: Omit<Issue, "resource">[] = [];
  const name = nameOf(resource.doc);
  if (isHole(name)) issues.push({ severity: "error", code: "hole", message: `metadata.name: to fill in (${name})`, path: "metadata.name" });
  else if (!NAME.test(name) || name.length > 253) {
    issues.push({ severity: "error", code: "invalid-name", message: `metadata.name: ${JSON.stringify(name)} is not a valid name`, path: "metadata.name" });
  }
  const namespace = namespaceOf(project, catalog, resource);
  if (namespace !== undefined && !NAME.test(namespace)) {
    issues.push({ severity: "error", code: "missing-namespace", message: `metadata.namespace: ${JSON.stringify(namespace)} is not a valid namespace`, path: "metadata.namespace" });
  }
  const gk = groupKindOf(resource.doc);
  const schema = gk && catalog.get(gkKey(gk))?.schema;
  if (resource.mode === "editable" && schema) {
    const spec = resource.doc.spec ?? {};
    for (const missing of findMissingRequired(schema, spec)) {
      issues.push({ severity: "error", code: "missing-required", message: `spec.${missing}: required`, path: `spec.${missing}` });
    }
    issues.push(...checkValue(schema, spec, "spec"));
  }
  return issues.map((issue) => ({ ...issue, resource: resource.id }));
}

function checkSlot(graph: Graph, project: Project, catalog: Catalog, slot: Slot): Issue[] {
  const path = formatPath(slot.path);
  const issue = (severity: Issue["severity"], code: Issue["code"], message: string): Issue => ({
    severity,
    code,
    message: `${path}: ${message}`,
    resource: slot.resource,
    path,
  });
  const { ref, target } = slot;
  if (!ref) return [];
  // A blank is the schema check's to report, on the field itself.
  if (isHole(ref.name)) return [];
  const kinds = slot.link.to.map((k) => k.kind).join(" or ");
  const source = graph.nodes.find((n) => n.id === slot.resource);
  // A cluster-scoped object has no namespace to default to.
  if (!target && source?.namespace === undefined && !ref.namespace && slot.link.to.every((to) => isNamespaced(catalog, to))) {
    return [issue("error", "missing-namespace", `a cluster-scoped object names the namespace of its ${kinds}`)];
  }
  if (!target) {
    const other = graph.nodes.find((n) => n.name === ref.name && (n.namespace === undefined || n.namespace === slot.namespace));
    return other
      ? [issue("error", "wrong-kind", `${ref.name} is a ${other.kind.kind}, not a ${kinds}`)]
      : [issue("warning", "dangling-reference", `no ${kinds} ${ref.name} in the project: declare it as external if it exists elsewhere`)];
  }
  const issues: Issue[] = [];
  if (ref.key && target.keys && !target.keys.includes(ref.key)) {
    issues.push(issue("warning", "missing-key", `${target.name} has no key ${ref.key} (known: ${target.keys.join(", ") || "none"})`));
  }
  const notShared = sharingRefusal(project, catalog, source, target);
  if (notShared) issues.push(issue("error", "not-shared", notShared));
  return issues;
}

/**
 * The kit's local instances share themselves with other namespaces
 * (`spec.sharedWith`): a reference from another namespace, or from a
 * cluster-scoped object, needs it. A selector cannot be checked here.
 */
function sharingRefusal(
  project: Project,
  catalog: Catalog,
  source: GraphNode | undefined,
  target: GraphNode,
): string | undefined {
  if (target.origin !== "resource" || target.namespace === undefined || source?.namespace === target.namespace) return undefined;
  if (!catalog.get(gkKey(target.kind))?.schema?.properties?.sharedWith) return undefined;
  const spec = project.resources.find((r) => r.id === target.id)?.doc.spec;
  const shared = isRecord(spec) && isRecord(spec.sharedWith) ? spec.sharedWith : {};
  if (shared.namespaceSelector !== undefined) return undefined;
  if (source?.namespace === undefined) {
    return shared.clusterScoped === true
      ? undefined
      : `${target.name} is not shared with cluster-scoped objects (spec.sharedWith.clusterScoped)`;
  }
  const namespaces = Array.isArray(shared.namespaces) ? shared.namespaces : [];
  return namespaces.includes(source.namespace)
    ? undefined
    : `${target.namespace}/${target.name} is not shared with namespace ${source.namespace} (spec.sharedWith)`;
}

function duplicates(graph: Graph): Issue[] {
  const seen = new Map<string, string>();
  const issues: Issue[] = [];
  for (const node of graph.nodes) {
    if (node.origin === "external") continue;
    const key = `${gkKey(node.kind)}/${node.namespace ?? ""}/${node.name}`;
    const first = seen.get(key);
    if (first === undefined) seen.set(key, node.id);
    else {
      issues.push({
        severity: "error",
        code: "duplicate",
        message: `${node.kind.kind} ${node.namespace ? `${node.namespace}/` : ""}${node.name} is already defined`,
        resource: node.from ?? node.id,
      });
    }
  }
  return issues;
}

export function validate(project: Project, catalog: Catalog, graph = buildGraph(project, catalog)): Issue[] {
  return [
    ...project.resources.flatMap((r) => checkResource(project, catalog, r)),
    ...graph.slots.flatMap((slot) => checkSlot(graph, project, catalog, slot)),
    ...duplicates(graph),
  ];
}
