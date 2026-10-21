// Mirrors the subset of a Kubernetes structural schema that `weebo-crd-docs`
// actually emits into public/crd-schemas/*.schema.json (k8s-only extensions
// already stripped there). Not general JSON Schema.

export type FieldType = "string" | "boolean" | "integer" | "number" | "object" | "array";

export interface FieldSchema {
  type?: FieldType;
  description?: string;
  nullable?: boolean;
  default?: unknown;
  enum?: string[];
  format?: string;
  items?: FieldSchema;
  properties?: Record<string, FieldSchema>;
  required?: string[];
  additionalProperties?: FieldSchema | boolean;
}

export interface CrdSchema extends FieldSchema {
  apiVersion: string;
  kind: string;
  scope: "Namespaced" | "Cluster";
}

/** One entry of crd-schemas/index.json, the kind list `CrdBuilder` offers. */
export interface CrdIndexEntry {
  kind: string;
  singular: string;
  scope: "Namespaced" | "Cluster";
}

// Fetched JSON is checked at the boundary on its top-level shape; the nested
// field schemas are `weebo-crd-docs`' own output and read as they come.
const isScope = (v: unknown): v is CrdSchema["scope"] => v === "Namespaced" || v === "Cluster";

export function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

export function isCrdSchema(v: unknown): v is CrdSchema {
  return isRecord(v) && typeof v.apiVersion === "string" && typeof v.kind === "string" && isScope(v.scope);
}

export function isCrdIndex(v: unknown): v is CrdIndexEntry[] {
  return (
    Array.isArray(v) &&
    v.every((e) => isRecord(e) && typeof e.kind === "string" && typeof e.singular === "string" && isScope(e.scope))
  );
}
