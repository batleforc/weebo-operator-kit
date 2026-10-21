// The kinds a project may hold: the published index, links and, once
// needed, schemas.

import { isRecord, type CrdSchema } from "../../crd-form/types";
import type { Catalog, GroupKind, KindEntry, KindLinks, LinksFile } from "./types";

export const gkKey = (gk: GroupKind): string => `${gk.group}/${gk.kind}`;

/** `group/version` (or `v1`, the core group) split; `undefined` if not an apiVersion. */
export function parseApiVersion(apiVersion: string): { group: string; version: string } | undefined {
  const parts = apiVersion.split("/");
  if (parts.length === 1 && parts[0]) return { group: "", version: parts[0] };
  const [group, version] = parts;
  return parts.length === 2 && group && version ? { group, version } : undefined;
}

const isGroupKind = (v: unknown): v is GroupKind =>
  isRecord(v) && typeof v.group === "string" && typeof v.kind === "string";

const isStrings = (v: unknown): v is string[] => Array.isArray(v) && v.every((s) => typeof s === "string");
const optional = (v: unknown, check: (v: unknown) => boolean) => v === undefined || check(v);
const isString = (v: unknown) => typeof v === "string";

function isKindLinks(v: unknown): v is KindLinks {
  return (
    isRecord(v) &&
    Array.isArray(v.references) &&
    v.references.every((r) => isRecord(r) && typeof r.path === "string" && Array.isArray(r.to) && r.to.every(isGroupKind)) &&
    Array.isArray(v.produces) &&
    v.produces.every(
      (p) =>
        isRecord(p) &&
        isGroupKind(p.kind) &&
        isStrings(p.name) &&
        optional(p.each, isString) &&
        optional(p.namespace, isString) &&
        optional(p.keys, isString) &&
        optional(p.keyNames, isStrings) &&
        optional(p.unless, (u) => isRecord(u) && isString(u.path) && isString(u.equals)),
    )
  );
}

/** Fetched JSON, checked at the boundary. */
export function isLinksFile(v: unknown): v is LinksFile {
  return isRecord(v) && v.version === 1 && isRecord(v.kinds) && Object.values(v.kinds).every(isKindLinks);
}

const NO_LINKS: KindLinks = { references: [], produces: [] };

/**
 * The catalog of the given kinds (an `index.json`'s entries, or schemas,
 * which say as much). `links` is keyed by kind alone, as one site's
 * `links.json` is: pass each site's kinds with its own file. `schemas` are
 * loaded already.
 */
export function buildCatalog(sources: { kinds: KindEntry[]; links: LinksFile; schemas?: CrdSchema[] }[]): Catalog {
  const catalog: Catalog = new Map();
  for (const { kinds, links, schemas = [] } of sources) {
    for (const entry of kinds) {
      const parsed = parseApiVersion(entry.apiVersion);
      if (!parsed) throw new Error(`${entry.kind}: invalid apiVersion ${JSON.stringify(entry.apiVersion)}`);
      const key = gkKey({ group: parsed.group, kind: entry.kind });
      if (catalog.has(key)) throw new Error(`${key} is in the catalog twice`);
      const schema = schemas.find((s) => s.kind === entry.kind && s.apiVersion === entry.apiVersion);
      catalog.set(key, {
        kind: entry.kind,
        group: parsed.group,
        version: parsed.version,
        apiVersion: entry.apiVersion,
        scope: entry.scope,
        served: entry.served ?? [parsed.version],
        links: links.kinds[entry.kind] ?? NO_LINKS,
        ...(schema ? { schema } : {}),
      });
    }
  }
  return catalog;
}

/** The catalog with these schemas loaded; one of no kind of it is ignored. */
export function withSchemas(catalog: Catalog, schemas: CrdSchema[]): Catalog {
  const next: Catalog = new Map(catalog);
  for (const schema of schemas) {
    const parsed = parseApiVersion(schema.apiVersion);
    const key = parsed && gkKey({ group: parsed.group, kind: schema.kind });
    const entry = key && next.get(key);
    if (key && entry) next.set(key, { ...entry, schema });
  }
  return next;
}

/** The kinds of the project's resources whose schema is not loaded yet. */
export function missingSchemas(catalog: Catalog, kinds: GroupKind[]): string[] {
  return [...new Set(kinds.map(gkKey))].filter((key) => {
    const entry = catalog.get(key);
    return entry !== undefined && entry.schema === undefined;
  });
}
