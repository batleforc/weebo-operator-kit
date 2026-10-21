import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { evaluate } from "@mdx-js/mdx";
import { createElement } from "react";
import * as runtime from "react/jsx-runtime";
import { renderToStaticMarkup } from "react-dom/server";
import { remarkHeading } from "fumadocs-core/mdx-plugins";
import * as yaml from "js-yaml";
import remarkGfm from "remark-gfm";
import { codeSpan, generateCrdDocs, mdxText, shortDescription } from "../src/crd-docs/generate.mjs";

const fixtures = path.join(path.dirname(fileURLToPath(import.meta.url)), "fixtures");
const crdDir = path.join(fixtures, "crd");

const config = {
  crdDir,
  kindPrefix: "Example",
  order: ["ExampleWidget"],
  guides: { ExampleWidget: ["Widgets", "/docs/guides/widgets"] },
  examples: {
    ExampleWidget: { name: "w" },
    ExampleGadget: { url: "https://gadget.example.com" },
    ExampleNested: { target: { host: "h" } },
  },
};

function run(overrides = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "crd-docs-"));
  const count = generateCrdDocs({ ...config, ...overrides }, { root });
  const read = (p) => fs.readFileSync(path.join(root, p), "utf8");
  return { root, count, read };
}

test("first sentence, across wrapped lines, skipping i.e./e.g.", () => {
  assert.equal(
    shortDescription("A widget, i.e. the remote object. It is mirrored\nas-is."),
    "A widget, i.e. the remote object.",
  );
  assert.equal(shortDescription("Lead-in:\n\n- a list"), "Lead-in.");
});

test("one page and one schema per kind, plus the section index", () => {
  const { root, count, read } = run();
  assert.equal(count, 3);
  const page = read("content/docs/crds/examplewidget.mdx");
  assert.match(page, /^---\ntitle: ExampleWidget\ndescription: A widget, i\.e\. the remote object\.\nfull: true\n---/);
  assert.match(page, /- \*\*API version:\*\* `example\.weebo\.io\/v1alpha1`/);
  assert.match(page, /- \*\*Short names:\*\* `ewg`/);
  assert.match(page, /- \*\*Guide:\*\* \[Widgets\]\(\/docs\/guides\/widgets\)/);
  assert.match(page, /name: my-widget\n  namespace: team-a/);
  assert.match(page, /\| `replicas` \| `integer` \| no \| `1` \|/);
  assert.match(page, /<CrdForm kind="ExampleWidget" \/>/);

  const schema = JSON.parse(read("public/crd-schemas/examplewidget.schema.json"));
  assert.equal(schema.apiVersion, "example.weebo.io/v1alpha1");
  assert.equal(schema.scope, "Namespaced");
  assert.ok(!("x-kubernetes-preserve-unknown-fields" in schema.properties.extra));

  assert.deepEqual(JSON.parse(read("content/docs/crds/meta.json")).pages, [
    "index",
    "examplewidget",
    "examplegadget",
    "examplenested",
  ]);
  assert.deepEqual(JSON.parse(read("public/crd-schemas/index.json")), [
    { kind: "ExampleWidget", singular: "examplewidget", scope: "Namespaced" },
    { kind: "ExampleGadget", singular: "examplegadget", scope: "Cluster" },
    { kind: "ExampleNested", singular: "examplenested", scope: "Namespaced" },
  ]);
  const index = read("content/docs/crds/index.mdx");
  assert.match(index, /\| Kind \| Short names \| Scope \| Description \|/);
  assert.match(index, /\| \[`ExampleGadget`\]\(\/docs\/crds\/examplegadget\) \|  \| Cluster \| A gadget: cluster-wide\. \|/);
  fs.rmSync(root, { recursive: true });
});

test("stale outputs are removed, the intro is configurable", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "crd-docs-"));
  fs.mkdirSync(path.join(root, "content/docs/crds"), { recursive: true });
  fs.writeFileSync(path.join(root, "content/docs/crds/gone.mdx"), "stale");
  generateCrdDocs({ ...config, indexIntro: "Custom intro." }, { root });
  assert.ok(!fs.existsSync(path.join(root, "content/docs/crds/gone.mdx")));
  assert.match(fs.readFileSync(path.join(root, "content/docs/crds/index.mdx"), "utf8"), /\n\nCustom intro\.\n\n\| Kind/);
  fs.rmSync(root, { recursive: true });
});

test("an example that drifted from the schema fails generation", () => {
  assert.throws(
    () => run({ examples: { ...config.examples, ExampleWidget: { name: "w", nope: 1 } } }),
    /ExampleWidget example: unknown field spec\.nope/,
  );
  assert.throws(
    () => run({ examples: { ...config.examples, ExampleGadget: {} } }),
    /ExampleGadget example: missing required spec\.url/,
  );
  assert.throws(() => run({ examples: { ExampleWidget: { name: "w" } } }), /no example spec for ExampleGadget/);
});

test("mdxText escapes outside code, keeps code spans as written", () => {
  assert.equal(mdxText("a {b} <c> \\d"), "a \\{b\\} \\<c\\> \\\\d");
  assert.equal(mdxText("`{x}` and `` a`b `` then {y}"), "`{x}` and `` a`b `` then \\{y\\}");
  assert.equal(mdxText("orphan ` {z}"), "orphan \\` \\{z\\}");
  assert.equal(mdxText("a | `b|c`\nd", { cell: true }), "a \\| `b\\|c` d");
  assert.equal(mdxText("plain text, unchanged"), "plain text, unchanged");
  assert.equal(mdxText("see ![x](u) and `![y](v)`"), "see \\![x](u) and `![y](v)`");
  assert.equal(mdxText("# Examples\n  # b, issue #3 `x`\n# c"), "\\# Examples\n  \\# b, issue #3 `x`\n\\# c");
});

test("mdxText cuts paragraphs before it looks for code", () => {
  assert.equal(mdxText("one\ntwo\n\nthree", { cell: true }), "one two<br /><br />three");
  assert.equal(mdxText("one\n\ntwo"), "one\n\ntwo");
  // A fence that spans paragraphs is no code span: what it encloses is escaped.
  assert.equal(mdxText("```\n<q>\n\n<r>\n```", { cell: true }), "\\`\\`\\` \\<q\\><br /><br />\\<r\\> \\`\\`\\`");
});

test("codeSpan fences past the longest backtick run", () => {
  assert.equal(codeSpan("plain"), "`plain`");
  assert.equal(codeSpan("a`b"), "``a`b``");
  assert.equal(codeSpan("`edge`"), "`` `edge` ``");
  assert.equal(codeSpan("a|b", { cell: true }), "`a\\|b`");
  assert.equal(codeSpan("multi\n  line"), "`multi line`");
});

test("nested objects get a section, an anchor and a link", async () => {
  const { root, read } = run();
  const page = read("content/docs/crds/examplenested.mdx");
  const sections = [...page.matchAll(/^### (.*) \[#(.*)\]$/gm)].map((m) => [m[1], m[2]]);
  assert.deepEqual(sections, [
    ["`spec.target`", "spec.target"],
    ["`spec.rules[]`", "spec.rules"],
    ["`spec.matrix[][]`", "spec.matrix"],
    ["`spec.byName[key]`", "spec.byName"],
    ["`spec.source`", "spec.source"],
    ["`spec.source.Secret`", "spec.source.Secret"],
    ["`spec.only`", "spec.only"],
    ["`spec.shape`", "spec.shape"],
  ]);
  assert.match(page, /\| \[`target`\]\(#spec\.target\) \| `object` \| yes \|/);
  assert.match(page, /\| `labels` \| `map\[string\]` \|/);
  assert.match(page, /\| \[`byName`\]\(#spec\.byName\) \| `map\[object\]` \|/);
  assert.match(page, /\| `port` \| `int-or-string` \|/);
  assert.match(page, /When to pull\.<br \/><br \/>Values: `"Always"`, `"IfNotPresent"` \|/);
  assert.match(page, /Constraints: `minimum: 1`, `maximum: 65535`/);
  assert.match(page, /\| `ConfigMap` \| `string` \| one of \|/);
  assert.match(page, /Exactly one of `Secret`, `ConfigMap`\./);
  assert.match(page, /### `spec\.only` \[#spec\.only\]\n\n[^#]*\| `Secret` \| `string` \| yes \|/);
  assert.doesNotMatch(page, /Exactly one of `Secret`\./);
  assert.match(page, /Matches one of 2 shapes; see the schema\./);
  assert.match(page, /- set a target or rules — `has\(self\.target\) \|\| has\(self\.rules\)` \(on `\.target`\)/);
  assert.match(page, /- a rule with a backtick — ``self\.note != '`'``/);
  assert.match(page, /- `tags`: tags must be unique — /);
  assert.match(page, /- `tags\[\]`: `self\.size\(\) < 64`/);
  assert.match(page, /First paragraph, wrapped\.<br \/><br \/>Second paragraph\./);

  // The site reads the anchors: each heading gets its id, each link a target.
  const html = await render(page);
  for (const [, anchor] of sections) {
    assert.match(html, new RegExp(`<h3 id="${anchor.replace(/\./g, "\\.")}"`));
  }
  for (const [, href] of html.matchAll(/href="#([^"]+)"/g)) assert.ok(html.includes(`id="${href}"`), href);
  // A kind with only scalars gets no section.
  assert.doesNotMatch(read("content/docs/crds/examplegadget.mdx"), /^### /m);
  fs.rmSync(root, { recursive: true });
});

/** A CRD directory holding one kind with `properties` as its spec fields. */
function crdWith(properties, names = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "crd-fixture-"));
  const crd = {
    spec: {
      group: "example.weebo.io",
      names: { kind: "ExampleBad", singular: "examplebad", ...names },
      scope: "Cluster",
      versions: [{ name: "v1", schema: { openAPIV3Schema: { properties: { spec: { type: "object", properties } } } } }],
    },
  };
  fs.writeFileSync(path.join(dir, "bad.yaml"), yaml.dump(crd));
  return dir;
}

test("duplicate anchors are refused, and no page is written", () => {
  const crdDir = crdWith({
    a: { type: "object", properties: { b: { type: "object", properties: { x: { type: "string" } } } } },
    "a.b": { type: "object", properties: { y: { type: "string" } } },
  });
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "crd-docs-"));
  fs.mkdirSync(path.join(root, "content/docs/crds"), { recursive: true });
  fs.writeFileSync(path.join(root, "content/docs/crds/kept.mdx"), "kept");
  assert.throws(
    () => generateCrdDocs({ crdDir, examples: { ExampleBad: {} } }, { root }),
    /ExampleBad: spec\.a\.b and spec\.a\.b would share the anchor #spec\.a\.b/,
  );
  assert.deepEqual(fs.readdirSync(path.join(root, "content/docs/crds")), ["kept.mdx"]);
  fs.rmSync(root, { recursive: true });
  fs.rmSync(crdDir, { recursive: true });
});

test("anchor charset, kind and singular are enforced", () => {
  const cases = [
    [crdWith({ "a`b": { type: "string" } }), /field spec\."a`b" is not in \[A-Za-z0-9_\.-\]/],
    [crdWith({ "a]b": { type: "object", properties: { x: { type: "string" } } } }), /field spec\."a]b"/],
    [crdWith({}, { kind: "Bad<Kind>" }), /invalid kind "Bad<Kind>"/],
    [crdWith({}, { singular: "../escape" }), /invalid singular "\.\.\/escape"/],
  ];
  for (const [crdDir, error] of cases) {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "crd-docs-"));
    assert.throws(() => generateCrdDocs({ crdDir, examples: { ExampleBad: {}, "Bad<Kind>": {} } }, { root }), error);
    fs.rmSync(root, { recursive: true });
    fs.rmSync(crdDir, { recursive: true });
  }
});

// Render a generated page the way the site does: frontmatter stripped
// (fumadocs-mdx reads it apart), GFM tables, `[#id]` headings, components
// stubbed. fumadocs-mdx's remarkImage is left out: the test asserts instead
// that no image comes out of CRD text.
async function render(source) {
  const body = source.replace(/^---\n[\s\S]*?\n---\n/, "");
  const { default: Content } = await evaluate(body, { ...runtime, remarkPlugins: [remarkGfm, remarkHeading] });
  return renderToStaticMarkup(createElement(Content, { components: { CrdForm: () => null } }));
}

test("crd text never reaches MDX as code", async () => {
  const pwnedFile = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "crd-pwned-")), "pwned");
  process.env.CRD_PWNED_FILE = pwnedFile;
  // Control: the same text, unescaped, does run at build time.
  await render("{globalThis.crdPwned = true}");
  assert.equal(globalThis.crdPwned, true);
  delete globalThis.crdPwned;

  const { root, read } = run({
    crdDir: path.join(fixtures, "crd-hostile"),
    examples: { ExampleHostile: { note: "n" } },
  });
  const page = await render(read("content/docs/crds/examplehostile.mdx"));
  const index = await render(read("content/docs/crds/index.mdx"));

  assert.equal(globalThis.crdPwned, undefined);
  assert.equal(globalThis.crdPwnedNested, undefined);
  assert.ok(!fs.existsSync(pwnedFile));
  for (const html of [page, index]) {
    assert.doesNotMatch(html, /<script|<img|<prefix/);
  }
  assert.match(page, /\{globalThis\.crdPwned = true\}/);
  assert.match(page, /\/apis\/\{group\}\/\{version\} and &lt;prefix&gt;\/&lt;namespace&gt;, a \{\{username\}\}/);
  assert.match(page, /a \\\{ and &lt;script&gt;alert\(1\)&lt;\/script&gt;/);
  assert.match(page, /<code>\{\{username\}\}<\/code> and <code>a`b<\/code>/);
  assert.match(page, /An orphan ` backtick \{x\}/);
  // The pipes stay inside their cells: five columns, `x|y` whole.
  const row = page.match(/<tr>\s*<td><code>note<\/code><\/td>[\s\S]*?<\/tr>/)[0];
  assert.equal(row.match(/<td>/g).length, 5);
  assert.match(row, /<code>&quot;x\|y&quot;<\/code>/);
  assert.match(row, /a \| b, &lt;img src=x onerror=alert\(1\)&gt; and \{1\+1\}\./);
  // Every row of every table keeps its five columns.
  for (const tr of page.match(/<tr>[\s\S]*?<\/tr>/g).slice(1)) {
    if (!tr.includes("<th")) assert.equal(tr.match(/<td>/g).length, 5, tr);
  }
  // Code-shaped CRD text: the fence outgrows the backticks inside.
  assert.match(page, /<code>&quot;a`\{b\}&quot;<\/code>/);
  assert.match(page, /<code>&quot;p\|q&quot;<\/code>, <code>&quot;r`s&quot;<\/code>/);
  assert.match(page, /<code>pattern: \^\[\{\]a\|&lt;b&gt;\$<\/code>/);
  assert.match(page, /<strong>Short names:<\/strong> <code>h`x<\/code>/);
  assert.match(index, /<td><code>h`x<\/code><\/td>/);
  // Images become a `!` and a link: nothing fetched at build, no pixel.
  assert.match(page, /An image: !<a href="https:\/\/example\.invalid\/p\.png">x<\/a>\./);
  // k8s-openapi's Quantity: a fence across paragraphs is text, `<…>` escaped.
  assert.match(page, /``` &lt;quantity&gt;/);
  // The same refusals in a nested section.
  assert.match(page, /<h3 id="spec\.nested">/);
  assert.match(page, /\{globalThis\.crdPwnedNested = true\} &lt;script&gt;alert\(2\)&lt;\/script&gt; !<a href/);
  fs.rmSync(root, { recursive: true });
  fs.rmSync(path.dirname(pwnedFile), { recursive: true });
});
