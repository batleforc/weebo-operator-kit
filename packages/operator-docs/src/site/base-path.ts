// Prefix for the static export when it's served under a sub-path (a GitHub
// Pages project site: /<repo>). Empty in dev and in a root-served build.
// `withOperatorDocs` (next.mjs) exposes the configured basePath as
// NEXT_PUBLIC_BASE_PATH, which Next inlines at build time — in this package's
// code too, since it is transpiled with the app. next/link and next/font
// prefix it themselves; plain `fetch`/`<a>` URLs to files in `public/` and to
// route handlers must go through `withBasePath`.
export const basePath = process.env.NEXT_PUBLIC_BASE_PATH ?? "";

export function withBasePath(path: string): string {
  return `${basePath}${path}`;
}
