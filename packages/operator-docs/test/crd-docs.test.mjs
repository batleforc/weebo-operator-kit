import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { generateCrdDocs, shortDescription } from "../src/crd-docs/generate.mjs";

const crdDir = path.join(path.dirname(fileURLToPath(import.meta.url)), "fixtures/crd");

const config = {
  crdDir,
  kindPrefix: "Example",
  order: ["ExampleWidget"],
  guides: { ExampleWidget: ["Widgets", "/docs/guides/widgets"] },
  examples: {
    ExampleWidget: { name: "w" },
    ExampleGadget: { url: "https://gadget.example.com" },
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
  assert.equal(count, 2);
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
  ]);
  assert.deepEqual(JSON.parse(read("public/crd-schemas/index.json")), [
    { kind: "ExampleWidget", singular: "examplewidget", scope: "Namespaced" },
    { kind: "ExampleGadget", singular: "examplegadget", scope: "Cluster" },
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
