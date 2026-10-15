import type { Metadata } from "next";
import type { ReactNode } from "react";
import { RootProvider } from "fumadocs-ui/provider/next";
import { withBasePath } from "./base-path";
import type { SiteConfig } from "./config";

/** Fumadocs' provider, set up for the theme and the static export. */
export function DocsRootProvider({ children }: { children: ReactNode }) {
  return (
    <RootProvider
      // `data-theme` is what the theme's tokens key off; `class` is what
      // Fumadocs' own `dark:` utilities key off. The stored value is the
      // preference (system|light|dark), resolved before first paint.
      theme={{ attribute: ["class", "data-theme"], defaultTheme: "system", enableSystem: true }}
      // `output: 'export'` has no server: the search index is a static file
      // (see `searchRoute`), queried in the browser.
      search={{ options: { type: "static", api: withBasePath("/api/search") } }}
    >
      {children}
    </RootProvider>
  );
}

/**
 * The root layout's metadata. `metadataBase` is the published site's absolute
 * URL (origin + base path): without it Next resolves OG image URLs against
 * localhost, which ships broken meta tags on the static export.
 */
export function siteMetadata(site: SiteConfig, metadataBase?: URL): Metadata {
  return {
    metadataBase,
    title: { default: site.name, template: `%s · ${site.name}` },
    description: site.description,
  };
}
