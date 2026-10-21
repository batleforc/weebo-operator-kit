// The CRD IDE's model: a project of manifests, the kinds it knows (the
// catalog) and what is derived from them (graph, issues). Pure data, no
// React: the views are projections of it.

import type { CrdSchema } from "../../crd-form/types";

/** A Kubernetes kind; `group` `""` is the core group. */
export interface GroupKind {
  group: string;
  kind: string;
}

/** A field naming another object (`weebo-crd-docs`' links.json). */
export interface Reference {
  path: string;
  to: GroupKind[];
}

/**
 * Objects a resource makes: one per value `each` reaches (default: the
 * resource itself), named by the first `name` field set. Paths other than
 * `each` are relative to each item; an item whose `unless.path` holds
 * `unless.equals` makes nothing.
 */
export interface Production {
  kind: GroupKind;
  each?: string;
  name: string[];
  /** The field naming its namespace; default the resource's. */
  namespace?: string;
  /** The field listing its keys. */
  keys?: string;
  /** Its keys, when fixed. */
  keyNames?: string[];
  unless?: { path: string; equals: string };
}

export interface KindLinks {
  references: Reference[];
  produces: Production[];
}

/** `crd-schemas/links.json`. */
export interface LinksFile {
  version: 1;
  kinds: Record<string, KindLinks>;
}

export interface CatalogKind {
  kind: string;
  group: string;
  /** The storage version, the one the IDE writes. */
  version: string;
  apiVersion: string;
  scope: "Namespaced" | "Cluster";
  /** Every served version. */
  served: string[];
  links: KindLinks;
  /**
   * Loaded the first time a resource of the kind needs it (`withSchemas`):
   * a catalog of a few hundred kinds holds megabytes of schemas. Until
   * then, the kind's specs are neither checked nor pruned.
   */
  schema?: CrdSchema;
}

/** What `index.json` says of a kind: enough to list it and name its version. */
export interface KindEntry {
  kind: string;
  apiVersion: string;
  scope: "Namespaced" | "Cluster";
  served?: string[] | undefined;
}

/** The kinds a project may hold, by `gkKey`. */
export type Catalog = Map<string, CatalogKind>;

/** A manifest, as parsed: checked to have `apiVersion`, `kind` and a name. */
export type Doc = Record<string, unknown>;

/**
 * `editable`: a kind of the catalog, in its storage version.
 * `readOnly`: a catalog kind in another version — kept, never rewritten.
 * `unknown`: not in the catalog — kept as is, exported unchanged.
 */
export type ResourceMode = "editable" | "readOnly" | "unknown";

export interface Resource {
  id: string;
  doc: Doc;
  mode: ResourceMode;
}

/** An object that exists outside the project (a Secret made by hand). Never exported. */
export interface External {
  id: string;
  kind: GroupKind;
  name: string;
  namespace?: string;
  keys: string[];
}

export interface Project {
  /** The namespace of a namespaced resource that names none. */
  defaultNamespace: string;
  resources: Resource[];
  externals: External[];
}

/** A step of a concrete path: a property name or an array index. */
export type Segment = string | number;

export type Severity = "error" | "warning" | "info";

export type IssueCode =
  | "invalid-document"
  | "unknown-kind"
  | "other-version"
  | "unsupported-version"
  | "invalid-name"
  | "missing-namespace"
  | "duplicate"
  | "missing-required"
  | "type"
  | "enum"
  | "pattern"
  | "range"
  | "unknown-field"
  | "hole"
  | "dangling-reference"
  | "wrong-kind"
  | "missing-key"
  | "not-shared";

export interface Issue {
  severity: Severity;
  code: IssueCode;
  message: string;
  /** The resource it is about; absent for a document refused at import. */
  resource?: string;
  /** Dotted path in the resource (`spec.instanceRef.name`). */
  path?: string;
}
