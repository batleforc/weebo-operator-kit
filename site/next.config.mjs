import { withOperatorDocs } from '@weebo/operator-docs/next';

// Fully static export (see withOperatorDocs). Served from a sub-path on GitHub
// Pages (`/weebo-operator-kit`); set by the site-pages workflow. Empty
// locally, so `task site` stays at the root.
export default withOperatorDocs({
  basePath: process.env.DOCS_BASE_PATH ?? '',
});
