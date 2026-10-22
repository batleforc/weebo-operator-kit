// The catalog, from the static files `weebo-crd-docs` writes: per source,
// its index.json and links.json up front; a kind's schema only once a
// resource of that kind is in the project (`loadSchemas`).

import { fetchJson } from "../crd-form/fetch-json";
import { isCrdIndex, isCrdSchema, type CrdSchema } from "../crd-form/types";
import { buildCatalog, gkKey, isLinksFile, parseApiVersion } from "./model/catalog";
import type { Catalog, KindEntry, LinksFile } from "./model/types";

/** A directory of the site holding `weebo-crd-docs`' schemas (`/crd-schemas`). */
export interface CatalogSource {
  base: string;
}

export const DEFAULT_SOURCES: CatalogSource[] = [{ base: "/crd-schemas" }];

const NO_LINKS: LinksFile = { version: 1, kinds: {} };

/** Where each kind's schema file is, by `gkKey`. */
export type SchemaFiles = Map<string, string>;

export async function loadCatalog(sources: CatalogSource[], signal: AbortSignal): Promise<{ catalog: Catalog; files: SchemaFiles }> {
  const files: SchemaFiles = new Map();
  const loaded = await Promise.all(
    sources.map(async ({ base }) => {
      const [index, links] = await Promise.all([
        fetchJson(`${base}/index.json`, isCrdIndex, signal),
        // A site generated before links.json existed has none: no reference known.
        fetchJson(`${base}/links.json`, isLinksFile, signal).catch((error: unknown) => {
          if (signal.aborted) throw error;
          return NO_LINKS;
        }),
      ]);
      const kinds = index.map((entry): KindEntry => {
        const parsed = entry.apiVersion ? parseApiVersion(entry.apiVersion) : undefined;
        if (!entry.apiVersion || !parsed) throw new Error(`${base}/index.json predates the IDE: regenerate it with weebo-crd-docs`);
        const key = gkKey({ group: parsed.group, kind: entry.kind });
        // The first source of a shared schematic kind, as the catalog keeps.
        if (!files.has(key)) files.set(key, `${base}/${entry.singular}.schema.json`);
        return { kind: entry.kind, apiVersion: entry.apiVersion, scope: entry.scope, served: entry.served, schematic: entry.schematic };
      });
      return { kinds, links };
    }),
  );
  return { catalog: buildCatalog(loaded), files };
}

/** The schemas of these kinds (`gkKey`s). */
export function loadSchemas(files: SchemaFiles, keys: string[], signal: AbortSignal): Promise<CrdSchema[]> {
  return Promise.all(
    keys.flatMap((key) => {
      const file = files.get(key);
      return file ? [fetchJson(file, isCrdSchema, signal)] : [];
    }),
  );
}
