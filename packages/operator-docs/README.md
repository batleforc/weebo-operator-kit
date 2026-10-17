# @weebo/operator-docs

The shared format of the weebo operators' documentation sites
([Fumadocs](https://fumadocs.dev) on Next.js, static export to GitHub Pages).
Each operator keeps its content and its business diagrams; everything else
comes from here:

| Entry point                        | What it gives the site                                                                                       |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `weebo-crd-docs` (bin)             | CRD YAML → `content/docs/crds/*.mdx` (+ `index.mdx`, `meta.json`) and `public/crd-schemas/*.json` (+ `index.json`); with a `reference` section, the operator reference pages too |
| `@weebo/operator-docs/crd-docs`    | the same generator as a function (`generateCrdDocs`), `defineConfig` for its config file                     |
| `@weebo/operator-docs/crd-form`    | the manifest builder: `CrdForm`, `CrdBuilder`, `ChoicePicker`, fields, schema utils, guided-choice types       |
| `@weebo/operator-docs/site`        | `defineSite`, `baseOptions`, `DocsRootProvider`, `siteMetadata`, `createDocsPage`, `OperatorHome`, `docsMdxComponents`, source helpers, `withBasePath` |
| `@weebo/operator-docs/routes`      | handlers of the search, `llms.txt`, `llms-full.txt`, per-page Markdown and OG image routes                   |
| `@weebo/operator-docs/config`      | the route constants alone (`docsRoute`, …), for `proxy.ts`                                                  |
| `@weebo/operator-docs/diagram-style` | SVG styles for the operator's own diagrams, on the theme's tokens                                        |
| `@weebo/operator-docs/next`        | `withOperatorDocs`, the whole `next.config.mjs`                                                              |
| `@weebo/operator-docs/css/theme.css` | the theme: design tokens, Fumadocs bridge, component rules                                              |

The package ships TypeScript sources: the site compiles them with its own
code (`withOperatorDocs` adds the package to `transpilePackages`).

## Installing

The package is released with the kit: every kit tag `vX.Y.Z` carries
`weebo-operator-docs-X.Y.Z.tgz` as a release asset (`.github/workflows/release.yml`).
In the operator's `docs/package.json`:

```json
"@weebo/operator-docs": "https://github.com/batleforc/weebo-operator-kit/releases/download/vX.Y.Z/weebo-operator-docs-X.Y.Z.tgz"
```

Its `fumadocs-core`, `fumadocs-ui`, `fumadocs-mdx` and `next` peers are
caret ranges (see `peerDependencies`): a site takes a security fix of Next
or Fumadocs within the major without waiting for a kit release. Raising a
range's floor or moving to a new major is a kit release, like moving `kube`
for the crates; `devDependencies` pin each floor, which `task docs:check`
type-checks against.

A site managed by pnpm sets `NPM_CLIENT: pnpm` on its `kit:` include, so
`task kit:sync` refreshes `pnpm-lock.yaml` instead of `package-lock.json`.

To iterate on the package and a site together, point the site at a sibling
checkout and install it as a copy, not a symlink (a symlinked package would
resolve its imports from the kit's directory, where the site's
dependencies are not):

```bash
# docs/package.json: "@weebo/operator-docs": "file:../../weebo-operator-kit/packages/operator-docs"
npm install --install-links
```

Re-run it after each change in the package. Never push an operator with a
`file:` dependency.

## The expected tree

```text
docs/
├── crd-docs.config.mjs          # input of weebo-crd-docs (examples, guides, order)
├── next.config.mjs              # withOperatorDocs({ basePath })
├── source.config.ts             # fumadocs-mdx: defineDocs({ dir: 'content/docs', includeProcessedMarkdown })
├── perf/<operator>.json         # the last `task perf` report (target/perf/), copied
├── content/docs/
│   ├── index.mdx, meta.json     # hand-written; lists "...crds" and "reference"
│   ├── guides/*.mdx, meta.json  # hand-written
│   ├── crds/                    # generated: <kind>.mdx, index.mdx, meta.json
│   └── reference/               # generated: reason-codes, events, metrics, resource-budget, meta.json
├── public/crd-schemas/          # generated: <kind>.schema.json, index.json
└── src/
    ├── app/                     # thin route files, see below
    ├── components/mdx.tsx       # docsMdxComponents(CHOICES) + the operator's own components
    └── lib/
        ├── shared.ts            # export const site = defineSite({ … })
        ├── source.ts            # loader({ source: docs.toFumadocsSource(), ...docsLoaderOptions })
        └── crd-choices.ts       # export const CHOICES: CrdChoices = { … }
```

The `crds/` section is entirely generated: `weebo-crd-docs` removes its
previous `.mdx`/`meta.json`/`.json` outputs before writing, so a CRD that
disappears takes its page with it. Never edit those files by hand.

## Wiring a site

`next.config.mjs` — a fully static export, Fumadocs MDX, the package
transpiled, the base path passed on to client code as
`NEXT_PUBLIC_BASE_PATH`:

```js
import { withOperatorDocs } from '@weebo/operator-docs/next';

export default withOperatorDocs({ basePath: process.env.DOCS_BASE_PATH ?? '' });
```

`src/app/global.css`:

```css
@import 'tailwindcss';
@import 'fumadocs-ui/css/neutral.css';
@import 'fumadocs-ui/css/preset.css';
@import '@weebo/operator-docs/css/theme.css';
```

`src/lib/shared.ts`:

```ts
import { defineSite } from '@weebo/operator-docs/site';

export const site = defineSite({
  name: 'weebo-<product>',
  description: 'One sentence: the default meta description.',
  git: { user: 'batleforc', repo: 'weebo-<product>', branch: 'main', contentDir: 'docs/content/docs' },
});
```

### Root layout

The fonts stay in the site (`next/font` only works in app code); the theme
reaches them through two CSS variables, `--font-silkscreen` (display) and
`--font-jetbrains-mono` (text):

```tsx
import './global.css';
import { JetBrains_Mono, Silkscreen } from 'next/font/google';
import { DocsRootProvider, siteMetadata } from '@weebo/operator-docs/site';
import { site } from '@/lib/shared';

const silkscreen = Silkscreen({ subsets: ['latin'], weight: ['400', '700'], variable: '--font-silkscreen' });
const jetbrainsMono = JetBrains_Mono({ subsets: ['latin'], variable: '--font-jetbrains-mono' });

export const metadata = siteMetadata(site, new URL('https://batleforc.github.io/weebo-<product>'));

export default function Layout({ children }: LayoutProps<'/'>) {
  return (
    <html lang="en" className={`${silkscreen.variable} ${jetbrainsMono.variable} font-sans`} suppressHydrationWarning>
      <body className="flex flex-col min-h-screen">
        <DocsRootProvider>{children}</DocsRootProvider>
      </body>
    </html>
  );
}
```

### Routes

Next reads route segment config (`revalidate`, `dynamic`) from the route
file itself, so it stays there; the handlers come from the package:

```ts
// src/app/docs/[[...slug]]/page.tsx
const docsPage = createDocsPage({ source, site, getMDXComponents });
export default docsPage.Page;
export const { generateStaticParams, generateMetadata } = docsPage;

// src/app/api/search/route.ts
export const revalidate = false;
export const GET = searchRoute(source);

// src/app/llms.txt/route.ts, src/app/llms-full.txt/route.ts
export const revalidate = false;
export const dynamic = 'force-static';
export const GET = llmsIndexRoute(source); // llmsFullRoute(source)

// src/app/llms.mdx/docs/[[...slug]]/route.ts, src/app/og/docs/[...slug]/route.tsx
export const revalidate = false;
export const dynamic = 'force-static';
export const { GET, generateStaticParams } = llmsPageRoute(source); // ogRoute(source, site)
```

The home and docs layouts spread `baseOptions(site)` into Fumadocs'
`HomeLayout`/`DocsLayout`; the landing page is `<OperatorHome>` with the
operator's own words (label, caption, install command, example manifest,
three features).

### MDX components

```tsx
// src/components/mdx.tsx
export function getMDXComponents(components?: MDXComponents) {
  return { ...docsMdxComponents(CHOICES), MyDiagram, ...components } satisfies MDXComponents;
}
```

`docsMdxComponents` brings Fumadocs' defaults, `Accordion(s)`, `Step(s)`,
`Tab(s)`, `<CrdForm kind="…" />` (on every generated page) and
`<CrdBuilder />` (a kind picker in front of the same form, for a "build a
manifest" guide), both bound to the operator's guided choices.

## The CRD reference generator

```bash
npm run gen:crds   # "gen:crds": "weebo-crd-docs" in the site's package.json; task recu runs it
```

`weebo-crd-docs [--config crd-docs.config.mjs]` reads the config from the
current directory; its relative paths resolve against the config file's
directory.

```js
import { defineConfig } from '@weebo/operator-docs/crd-docs';

export default defineConfig({
  // all optional, defaults shown
  crdDir: '../deploy/crd',
  contentDir: 'content/docs/crds',
  schemaDir: 'public/crd-schemas',
  baseUrl: '/docs/crds',
  exampleNamespace: 'team-a',
  // `<Prefix>Repository` → example named `my-repository`
  kindPrefix: '<Prefix>',
  // the paragraph above the index table (Markdown); a generic one by default
  indexIntro: '…',
  // sidebar order; kinds not listed come last, alphabetically
  order: ['<Prefix>Instance', '…'],
  // the guide explaining a kind, linked from its page
  guides: { '<Prefix>Instance': ['Connect an instance', '/docs/guides/connect-instance'] },
  // required for every kind: a minimal spec, checked against the schema
  // (every required field present, every key a real field)
  examples: { '<Prefix>Instance': { url: 'https://…' } },
});
```

Per kind, the page has its facts (API version, scope, short names, guide),
the spec's description (its first sentence becomes the page description),
the example manifest, the spec field table and the manifest builder. A field
without a description means the Rust doc-comment is missing: fix it there.
The index table shows a short-names column only when some kind has short
names.

## The operator reference

Four pages every operator needs and that the kit makes identical in
behaviour, so their text lives here and only their data is the
operator's. Add a `reference` section to `crd-docs.config.mjs` and the same
`weebo-crd-docs` run writes them to `content/docs/reference/`:

| Page | Generated from |
| ---- | -------------- |
| `reason-codes` | the operator's `reason_codes!` catalog (codes, doc comments, severities, the success code), and how its code uses them: admission denials (`Denial::new`), lifecycle Events (`announce_change`), the finalizer's release code (`controller::finalize`), the errors announced before a status (`(Code, events::…)`) |
| `events` | the kit's Event rules (`weebo-kit-runtime::events`), with those codes and the CRDs' group (the `force-orphan` annotation) |
| `metrics` | the kit's metrics, probes and runtime settings, with the operator's metric prefix (and its leader gauge when the sources declare `<prefix>_leader`) |
| `resource-budget` | the kit's `Budget::small_cluster()`, the operator's overrides, and the `weebo-kit-perf` report committed under `perf/` |

```js
reference: {
  // required; checked against the sources
  metricsPrefix: 'weebo_<product>',
  // optional, defaults shown
  dir: 'content/docs/reference',
  reasonCodes: '../crates/domain/src/error.rs',
  sources: '../crates',
  perfTask: 'task perf',
  // optional
  remedies: { '<Code>': 'What to do (Markdown)' },        // adds a "What to do" column
  shownAs: { '<Code>': 'where it shows, when the scan is wrong' },
  extraMetrics: [{ name: 'weebo_<product>_…', type: 'counter', labels: '`op`', description: '…' }],
  budget: { idle_rss_mib: 96 },                            // the perf test's overrides
  perfReport: 'perf/weebo-<product>.json',
  perfNote: 'kit vX.Y.Z, <date>',
},
```

The Rust sources are scanned as text, not compiled: the catalog must be
declared with the kit's `reason_codes!` and its severities in a `match`;
test files and `#[cfg(test)]` modules are left out. A metric name, a code
in `remedies` or a budget field that doesn't exist fails the run. Doc
comments are the "Meaning" column: write them for users, and keep pointers
for developers in a trailing `See …` sentence or a `(see …)` aside naming
a Rust path or a design note — those are dropped from the page.

The values the pages quote from the kit (`small-cluster-budget.json`,
`kit-runtime.json` under `src/reference/`) are copies, checked equal to the
Rust constants by tests in `weebo-kit-perf` and `weebo-kit-runtime`.

## Guided choices

Some kinds accept mutually exclusive ways of doing one thing (credentials in
a Secret or in Vault, a resource created empty or from a template). The
schema can only say "all optional"; the operator's `CrdChoices` table says
which fields go together, so the form asks "how?" first:

```ts
export const CHOICES: CrdChoices = {
  '<Prefix>Repository': [
    {
      id: 'source',
      label: 'Start from',
      options: [
        { id: 'init', label: 'Empty', hint: '…', fields: ['init'] },
        { id: 'template', label: 'Template', hint: '…', fields: ['fromTemplate'], require: ['fromTemplate'] },
      ],
    },
  ],
};
```

Per option: `fields` shown (and kept in the manifest) only for it,
`require` (dotted paths allowed) marked required, `set` implied values whose
fields are hidden, `preset` values written in but left editable.

## Base path

The export is served under `/<repo>` on GitHub Pages. Next prefixes links
and assets itself, not a runtime `fetch()` of a file in `public/`: go
through `withBasePath()` (the manifest builder and the search client
already do).

## Developing the package

```bash
task docs:check   # version in step with the workspace, tsc against the peers' floors, generator tests
task docs:pack    # dist/weebo-operator-docs-<version>.tgz, what the release attaches
```

The package's version is the workspace's (`.hooks/bump-versions.sh`, run by
`cog bump`); `docs:check` fails when they differ. Nothing here names an
operator: what varies is a parameter (`defineSite`, the generator's config,
`CrdChoices`).
