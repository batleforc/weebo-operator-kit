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
  pattern?: string;
  minimum?: number;
  maximum?: number;
  minLength?: number;
  maxLength?: number;
  minItems?: number;
  maxItems?: number;
}

export interface CrdSchema extends FieldSchema {
  apiVersion: string;
  kind: string;
  scope: "Namespaced" | "Cluster";
  /** Every served version; `apiVersion` is the storage one. */
  served?: string[];
}

/** One entry of crd-schemas/index.json, the kind list `CrdBuilder` offers. */
export interface CrdIndexEntry {
  kind: string;
  singular: string;
  scope: "Namespaced" | "Cluster";
  /** The storage version's; absent from a site generated before the IDE. */
  apiVersion?: string;
  served?: string[];
  /** A kind the CRD IDE draws but Kubernetes never sees: not a CRD. */
  schematic?: boolean;
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
    v.every(
      (e) =>
        isRecord(e) &&
        typeof e.kind === "string" &&
        typeof e.singular === "string" &&
        isScope(e.scope) &&
        (e.apiVersion === undefined || typeof e.apiVersion === "string") &&
        (e.served === undefined || (Array.isArray(e.served) && e.served.every((s) => typeof s === "string"))) &&
        (e.schematic === undefined || typeof e.schematic === "boolean"),
    )
  );
}
