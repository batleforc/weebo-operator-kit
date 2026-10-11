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
