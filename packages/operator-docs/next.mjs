// `next.config.mjs` of every operator's docs site:
//
//   import { withOperatorDocs } from '@weebo/operator-docs/next';
//   export default withOperatorDocs({ basePath: process.env.DOCS_BASE_PATH ?? '' });
//
// Plain JavaScript: Next loads its config with Node, untranspiled.
import { createMDX } from "fumadocs-mdx/next";

/**
 * A fully static site (`output: 'export'`, GitHub Pages has no Node server)
 * with Fumadocs MDX, this package transpiled with the app (it ships its
 * TypeScript sources), and the base path exposed to client code.
 *
 * @param {{ basePath?: string } & import('next').NextConfig} [options]
 *   `basePath`: the sub-path the export is served under (a GitHub Pages
 *   project site: `/<repo>`); empty locally, so `next dev` serves at `/`.
 *   Anything else is merged into the Next config.
 * @returns {import('next').NextConfig}
 */
export function withOperatorDocs({ basePath = "", ...config } = {}) {
  const withMDX = createMDX();
  return withMDX({
    reactStrictMode: true,
    output: "export",
    // `/docs/install/index.html` rather than `/docs/install.html`, which is
    // what a static host resolves for `/docs/install/`.
    trailingSlash: true,
    // next/image optimisation needs a server.
    images: { unoptimized: true },
    basePath,
    ...config,
    // Next rewrites framework URLs (links, assets) under basePath, but not a
    // runtime `fetch()` of a file in `public/`: `withBasePath` reads this.
    env: { NEXT_PUBLIC_BASE_PATH: basePath, ...config.env },
    transpilePackages: ["@weebo/operator-docs", ...(config.transpilePackages ?? [])],
  });
}
