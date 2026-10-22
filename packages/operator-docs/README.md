# @weebo/operator-docs

The shared format of the weebo operators' documentation sites
([Fumadocs](https://fumadocs.dev) on Next.js, static export to GitHub Pages).
Each operator keeps its content and its business diagrams; everything else
comes from here:

| Entry point                        | What it gives the site                                                                                       |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `weebo-crd-docs` (bin)             | CRD YAML → `content/docs/crds/*.mdx` (+ `index.mdx`, `meta.json`) and `public/crd-schemas/*.json` (+ `index.json`, `links.json`); with a `reference` section, the operator reference pages too |
| `@weebo/operator-docs/crd-docs`    | the same generator as a function (`generateCrdDocs`), `defineConfig` for its config file                     |
| `@weebo/operator-docs/crd-form`    | the manifest builder: `CrdForm`, `CrdBuilder`, `ChoicePicker`, fields, schema utils, guided-choice types       |
| `@weebo/operator-docs/crd-ide/model` | the CRD IDE's pure model: catalog, project, graph of references, validation, YAML import/export, zip, layout |
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
`Tab(s)`, `<CrdForm kind="…" />` (on every generated page),
`<CrdBuilder />` (a kind picker in front of the same form, for a "build a
manifest" guide) and `<CrdIde />` (see below), all bound to the operator's
guided choices.

## The CRD IDE

`<CrdIde />` composes a project of manifests linked to each other: an
instance, the Secret its token comes from, the objects that live on it.
Put it on a page of its own (`full: true` in the frontmatter): its code
loads only there, and the graph's `@xyflow/react` with the project's first
resource.

```tsx
// src/components/mdx.tsx — schema directories (weebo-crd-docs output);
// the default is the site's own /crd-schemas
docsMdxComponents(CHOICES, { sources: [{ base: '/crd-schemas' }] });
```

```mdx
{/* content/docs/ide.mdx — recipes: ready-made projects, multi-document
    YAML with `<…>` marking what to fill in. Passed here, on the IDE's page:
    given to docsMdxComponents, they would ship with every page. */}
import { RECIPES } from '@/lib/recipes';

<CrdIde recipes={RECIPES} />
```

A recipe is `{ id, label, description?, yaml }`.

- **Resources**: add one of any catalog kind, import YAML (files or
  paste), or add a recipe. A manifest in another served version of a kind,
  or of a kind outside the catalog, is kept read-only and exported as is.
- **Import a cluster's objects**: `kubectl get <kinds> -A -o yaml` (a
  `List`) imports as is, several files at once. What the server sets
  (`status`, `uid`, `resourceVersion`, `managedFields`, the
  last-applied annotation…) is dropped; a `Secret` becomes an external
  object with its keys, never its values (neither in the page nor in the
  draft); an object already in the project is replaced, not duplicated.
- **Links** come from `links.json` (see [Links](#links)): under each
  resource, every reference field is a select of the objects it may point
  at — the project's resources, the objects they make (the Secret of an
  external secret) and the external objects declared by hand (never
  exported). The graph, above, shows them; the list is the keyboard way,
  with every action.
- **Schematic objects** (see [Schematic kinds](#schematic-kinds)): kinds
  an operator declares for the IDE alone — a cluster and the ways to reach
  it, the upstream a registry mirrors. Added, edited and linked as any
  resource, any name allowed (a URL), drawn with a dotted border and
  marked "(schematic)". A production may make them from a resource's
  fields: they follow its YAML, one node per name, merged into the object
  added by hand with that name. Left out of the manifests, downloaded
  apart as `schematic.yaml`, which imports back.
- **Issues**, as you type: the schema (required, type, enum, pattern,
  bounds, unknown fields), the recipe's blanks, references to nothing or to
  the wrong kind, a missing key, a local instance not shared with the
  referencing namespace, duplicates. CEL rules are left to the API server.
- **Out**: multi-document YAML in dependency order (copy or `.yaml`), or a
  `.zip` of one file per resource; the schematic objects in
  `schematic.yaml`. The project is kept in the browser's
  storage between visits.

The model behind it (`@weebo/operator-docs/crd-ide/model`) is plain
TypeScript, without React, for scripts and tests.

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
  // what the crdgen's links.json cannot say (see "Links" below)
  links: {},
});
```

Per kind, the page has its facts (API version, scope, short names, guide),
the spec's description (its first sentence becomes the page description),
the example manifest, the spec field table and the manifest builder. A field
without a description means the Rust doc-comment is missing: fix it there;
the generator prints a count per kind (`ForgejoRepository: 3 fields without
a description`). The index table shows a short-names column only when some
kind has short names.

Under the spec table, every nested object has its own section, depth first
in field order: its path as title (`spec.a.b`, `spec.a[]` for an array,
`spec.a[key]` for a map), its field table, and what the schema says about
it. The section's anchor is the path without `[]`/`[key]` (`#spec.a.b`),
linked from the parent's row and listed in the page's table of contents.
Property names must stay within `[A-Za-z0-9_.-]`, and two sections may not
share an anchor (a field `a.b` next to an object `a` with a field `b`):
either fails generation, and no page is written.

- **Type**: `map[string]`, `object[]`, `string[][]`, `int-or-string`, …
- **Required**: `yes`/`no`, or `one of`/`any of` for the fields of a serde
  enum (`oneOf` of single `required`), whose section then says `Exactly one
  of …`. Any other `oneOf`/`anyOf` gets a generic line.
- **Description**: the field's text, then `Values:` (enum, without `null`)
  and `Constraints:` (`pattern`, `minimum`, `maxLength`, `minItems`, …). A
  paragraph break stays one (`<br /><br />`).
- **Rules**: each `x-kubernetes-validations` entry, as its message, the rule
  and its `fieldPath`, under the object it applies to. A rule on a string or
  a list is listed under the object holding that field, after its name.

CRD text is written as inline Markdown: `*`, `` ` `` and `[links](…)` keep
their meaning, but outside code spans `\ { } < >` and the `!` of `![` are
escaped, so a description can never become a JS expression (evaluated at
build time), a JSX element or an image fetched at build time. Names,
defaults, enum values and rules are shown as code, fenced past any backtick
they contain.

### Links

`public/crd-schemas/links.json` tells the CRD IDE which fields name another
object and which objects a kind makes. It merges two sources:

- `<crdDir>/links.json`, written by the operator's `crdgen` from the
  markers of its `api` types (`weebo_kit_api::links`): `x-weebo-link` on a
  reference field, `x-weebo-produces` on the kit's `SecretTarget` (one
  Secret per Kubernetes target), `x-weebo-keys` on the field listing them
  (the keys those Secrets hold). An operator whose crdgen predates it has
  no links there.
- The config's `links` (`CrdLinks`), per kind: `references` a CRD written by
  another tool cannot carry, and `produces`, the objects a resource of the
  kind makes:

```js
links: {
  ExternalSecret: {
    produces: [{
      kind: { kind: 'Secret' },
      // the first field set names it
      name: ['spec.target.name', 'metadata.name'],
      // the field listing its keys; or `keyNames: ['tls.crt', …]`, fixed
      keys: 'spec.data[].secretKey',
    }],
  },
},
```

A production may also make one object per item (`each: 'spec.targets[]'`,
the other paths then relative to the item, `.` being the item itself when
it is a plain string), name its namespace's field (`namespace`, default the
resource's) and skip items (`unless: { path: 'backend', equals: 'vault'
}`; a boolean or a number compares as its text). Under a free-form object
(`x-kubernetes-preserve-unknown-fields`), any path is accepted: `each:
'spec.config.upstreams[]'`.

A name the operator computes is a `template`, used when no `name` field is
set — so a `name` field is the user's override: `{path}` relative to the
item, `{/path}` from the resource's root, `{/ref>path}` read on the
resource the reference field `ref` designates (unset: the only candidate,
as the kit's default instance). A hole without a value makes no object.

```js
// The issuer of an application: `issuerUrl` if set, else computed.
{ kind: { group: 'schematic.weebo.io', kind: 'OidcIssuer' }, each: 'spec.provider.oauth2',
  name: ['issuerUrl'], template: '{/spec.instanceRef>spec.url}/application/o/{/spec.slug}/' }
```

Paths are dotted from the object's root (`spec.instanceRef`), `[]` stepping
into an array's items and `*` into a map's values. The output is
`{ version: 1, kinds: { <Kind>: { references: [{ path, to: [{ group, kind }] }], produces: [...] } } }`,
`group: ""` being the core group. Generation fails, writing nothing, on a
kind that is not one of the CRDs (a stale `links.json`: rerun the crdgen),
a path that is not a field of the CRD, or a reference declared twice.

### Schematic kinds

A kind the IDE draws but Kubernetes never sees: no CRD, nothing to apply.

The kit declares a **shared vocabulary**, group `schematic.weebo.io`
(`weebo_kit_api::schematic`): `OidcIssuer`, `UpstreamRegistry`,
`VaultServer`, `HttpEndpoint`, `DnsName`, `S3Bucket` (cluster-scoped, named
by their URL or name) and `Database` (namespaced). The template's `crdgen`
declares them all (`schematic::shared`): every operator's site publishes
them, the IDE keeps the first, and two operators naming one issuer meet on
one node. A URL is compared without its trailing slash. What only one
operator draws gets a group of its own, as below.
The operator declares it in Rust, its `spec` a type with doc comments and
the same markers as a CRD's, and its `crdgen` writes it to `links.json`:

```rust
/// A cluster the proxy gives access to.
#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ClusterSpec {
    /// The API server's URL.
    api_server: String,
}

/// One way to reach a cluster.
#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
struct AccessMethodSpec {
    /// `oidc`, `token`…
    r#type: String,
    #[schemars(extend("x-weebo-link" = { "to": [{ "group": "schematic.example.weebo.io", "kind": "Cluster" }] }))]
    cluster_ref: ObjectRef,
}

// crates/crdgen/src/main.rs
let mut schematic = BTreeMap::new();
schematic.insert("Cluster".into(), links::schematic::<ClusterSpec>(GROUP, links::Scope::Cluster, &roles)?);
schematic.insert("AccessMethod".into(), links::schematic::<AccessMethodSpec>(GROUP, links::Scope::Namespaced, &roles)?);
```

A CRD's field may make schematic objects from its values, so the drawing
follows the manifests (`.`: the item itself):

```rust
/// One `Upstream` per URL of `config.upstreams`.
#[schemars(extend("x-weebo-produces" = { "kind": { "group": "schematic.example.weebo.io", "kind": "Upstream" }, "each": "upstreams[]", "name": ["."] }))]
pub config: serde_json::Value,
```

The generator publishes each one as a kind in version `v1` of its group:
a `<kind>.schema.json` and an `index.json` entry marked `schematic: true`
(`<CrdBuilder />` leaves them out), no page. Its links' paths start at the
object's root (`spec.clusterRef`). Generation fails on a group that is not
a DNS subdomain, a scope other than `Namespaced`/`Cluster`, no schema, or a
name already a CRD's. Use a group of the operator's own, apart from its
CRDs' (`schematic.<operator group>`): the kit's site loads every
operator's.

### Schemas only

`pages: false` writes `public/crd-schemas/` (schemas, `index.json`,
`links.json`) and no page, with no example needed: a catalog of CRDs
another tool owns, for the CRD IDE (the kit's site builds its catalog of
every operator's and some third-party CRDs this way). `index.json` lists
each kind's storage `apiVersion` and served versions: the IDE lists every
kind from it and fetches a kind's schema only once the project holds a
resource of that kind.

## The operator reference

Four pages every operator needs and that the kit makes identical in
behaviour, so their text lives here and only their data is the
operator's. Add a `reference` section to `crd-docs.config.mjs` and the same
`weebo-crd-docs` run writes them to `content/docs/reference/`:

| Page | Generated from |
| ---- | -------------- |
| `reason-codes` | the operator's `reason_codes!` catalog (codes, doc comments, severities, the success code), and how its code uses them: admission denials (`Denial::new`), lifecycle Events (`announce_change`), the finalizer's release code (`controller::finalize`), the errors announced before a status (`(Code, events::…)`) |
| `events` | the kit's Event rules (`weebo-kit-runtime::events`), with those codes and the CRDs' group (the `force-orphan` annotation) |
| `metrics` | the kit's metrics, probes and runtime settings, with the operator's metric prefix (its leader gauge when the sources declare `<prefix>_leader`, and `<prefix>_remote_writes_total` with its drift query only when they call `record_write`) |
| `resource-budget` | the kit's `Budget::small_cluster()`, the operator's overrides, and the `weebo-kit-perf` report committed under `perf/` |

```js
reference: {
  // required; checked against the sources
  metricsPrefix: 'weebo_<product>',
  // optional, defaults shown
  group: '<the CRDs' group>',                              // required without CRDs
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

`group` is the API group the pages name (the `force-orphan` annotation).
It defaults to the CRDs' group, and a value that differs from it fails the
run. A component without CRDs (only configuration and admission policies)
sets it: the reference pages are written, the CRD section is not (no
`crds/` page, no schema), and `crdDir` may be absent.

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
task docs:check   # version in step with the workspace, tsc against the peers' floors, generator tests, eslint
task docs:pack    # dist/weebo-operator-docs-<version>.tgz, what the release attaches
```

The package's version is the workspace's (`.hooks/bump-versions.sh`, run by
`cog bump`); `docs:check` fails when they differ. Nothing here names an
operator: what varies is a parameter (`defineSite`, the generator's config,
`CrdChoices`).
