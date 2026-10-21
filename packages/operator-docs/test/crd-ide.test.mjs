import assert from "node:assert/strict";
import { test } from "node:test";
import * as yaml from "js-yaml";
import {
  addExternal,
  addResource,
  buildCatalog,
  buildGraph,
  candidates,
  crc32,
  dependencyOrder,
  emptySlots,
  groupKindOf,
  missingSchemas,
  withSchemas,
  emptyProject,
  expand,
  exportFiles,
  exportYaml,
  formatPath,
  getAt,
  importYaml,
  isHole,
  isLinksFile,
  layout,
  nameOf,
  newDoc,
  parseApiVersion,
  parseLinkPath,
  pointAt,
  readRef,
  removeExternal,
  removeResource,
  setAt,
  setDefaultNamespace,
  updateResource,
  validate,
  zip,
} from "../src/crd-ide/model/index.ts";

const G = "example.weebo.io";
const secret = { group: "", kind: "Secret" };
const instance = { group: G, kind: "ExampleInstance" };
const local = { group: G, kind: "ExampleLocalInstance" };
const str = { type: "string" };

// Schemas as weebo-crd-docs publishes them: the spec, plus apiVersion/kind/scope/served.
const schemas = [
  {
    apiVersion: `${G}/v1alpha1`,
    kind: "ExampleInstance",
    scope: "Cluster",
    served: ["v1alpha1"],
    type: "object",
    required: ["url", "tokenSecretRef"],
    properties: {
      url: str,
      tokenSecretRef: { type: "object", required: ["name", "namespace", "key"], properties: { name: str, namespace: str, key: str } },
    },
  },
  {
    apiVersion: `${G}/v1alpha1`,
    kind: "ExampleLocalInstance",
    scope: "Namespaced",
    served: ["v1alpha1"],
    type: "object",
    required: ["tokenSecretRef"],
    properties: {
      tokenSecretRef: { type: "object", required: ["name", "key"], properties: { name: str, key: str } },
      sharedWith: {
        type: "object",
        properties: {
          namespaces: { type: "array", items: str },
          namespaceSelector: { type: "object" },
          clusterScoped: { type: "boolean" },
        },
      },
    },
  },
  {
    apiVersion: `${G}/v1alpha1`,
    kind: "ExampleObject",
    scope: "Namespaced",
    served: ["v1alpha1"],
    type: "object",
    properties: {
      instanceRef: {
        type: "object",
        nullable: true,
        required: ["name"],
        properties: { name: str, kind: { type: "string", enum: ["Instance", "LocalInstance"] }, namespace: str },
      },
      name: { type: "string", pattern: "^[a-z]+$", maxLength: 8 },
      replicas: { type: "integer", minimum: 1, maximum: 5 },
      tags: { type: "array", maxItems: 1, items: str },
      labels: { type: "object", additionalProperties: str },
      extra: { type: "object" },
    },
  },
];
const links = {
  version: 1,
  kinds: {
    ExampleInstance: { references: [{ path: "spec.tokenSecretRef", to: [secret] }], produces: [] },
    ExampleLocalInstance: { references: [{ path: "spec.tokenSecretRef", to: [secret] }], produces: [] },
    ExampleObject: { references: [{ path: "spec.instanceRef", to: [instance, local] }], produces: [] },
  },
};
// A third-party kind, from the catalog's other site.
const externalSchemas = [
    {
      apiVersion: "external-secrets.io/v1",
      kind: "ExternalSecret",
      scope: "Namespaced",
      served: ["v1", "v1beta1"],
      type: "object",
      properties: {
        target: { type: "object", properties: { name: str } },
        data: { type: "array", items: { type: "object", properties: { secretKey: str } } },
      },
    },
  ];
const external = {
  kinds: externalSchemas,
  schemas: externalSchemas,
  links: {
    version: 1,
    kinds: {
      ExternalSecret: {
        references: [],
        produces: [{ kind: secret, name: ["spec.target.name", "metadata.name"], keys: "spec.data[].secretKey" }],
      },
    },
  },
};
const catalog = buildCatalog([{ kinds: schemas, schemas, links }, external]);

const RECIPE = `
apiVersion: ${G}/v1alpha1
kind: ExampleObject
metadata: { name: docs, namespace: team-a }
spec:
  instanceRef: { name: main }
  name: docs
---
apiVersion: ${G}/v1alpha1
kind: ExampleLocalInstance
metadata: { name: main, namespace: team-a }
spec:
  tokenSecretRef: { name: example-token, key: token }
---
apiVersion: external-secrets.io/v1
kind: ExternalSecret
metadata: { name: example-token, namespace: team-a }
spec:
  data: [{ secretKey: token }]
`;

const load = (text, project = emptyProject("team-a")) => importYaml(project, catalog, text);
const codes = (issues) => issues.map((i) => i.code).sort();

test("paths: link paths reach every value, concrete paths read and write", () => {
  assert.deepEqual(parseLinkPath("spec.a[][].b.*"), [
    { prop: "spec" },
    { prop: "a" },
    { each: "item" },
    { each: "item" },
    { prop: "b" },
    { each: "value" },
  ]);
  assert.throws(() => parseLinkPath("spec..a"), /invalid link path/);
  const doc = { spec: { a: [{ ref: "x" }, {}, { ref: "y" }], m: { "k.1": { ref: "z" } } } };
  assert.deepEqual(expand(doc, "spec.a[].ref"), [
    { path: ["spec", "a", 0, "ref"], value: "x" },
    { path: ["spec", "a", 2, "ref"], value: "y" },
  ]);
  assert.deepEqual(expand(doc, "spec.m.*.ref"), [{ path: ["spec", "m", "k.1", "ref"], value: "z" }]);
  assert.deepEqual(expand(doc, "spec.nope[]"), []);
  assert.equal(getAt(doc, ["spec", "a", 2, "ref"]), "y");
  assert.equal(getAt(doc, ["spec", "a", "x"]), undefined);
  assert.equal(getAt(doc, ["spec", 0]), undefined);
  const written = setAt(doc, ["spec", "a", 1, "ref"], "w");
  assert.equal(getAt(written, ["spec", "a", 1, "ref"]), "w");
  assert.equal(getAt(doc, ["spec", "a", 1, "ref"]), undefined, "the input is untouched");
  const list = setAt(undefined, ["l", 1], "v");
  assert.equal(list.l.length, 2);
  assert.equal(list.l[1], "v");
  assert.equal(formatPath(["spec", "a", 1, "ref"]), "spec.a[1].ref");
});

test("catalog: links checked at the boundary, a kind once, apiVersions parsed", () => {
  assert.ok(isLinksFile(links));
  assert.ok(!isLinksFile({ version: 2, kinds: {} }));
  assert.ok(!isLinksFile({ version: 1, kinds: { X: { references: [{ path: 1, to: [] }], produces: [] } } }));
  assert.ok(!isLinksFile({ version: 1, kinds: { X: { references: [], produces: [{ kind: secret, name: "n" }] } } }));
  assert.throws(() => buildCatalog([{ kinds: schemas, links }, { kinds: schemas, links }]), /is in the catalog twice/);
  assert.throws(() => buildCatalog([{ kinds: [{ ...schemas[0], apiVersion: "a/b/c" }], links }]), /invalid apiVersion/);
  assert.deepEqual(parseApiVersion("v1"), { group: "", version: "v1" });
  assert.deepEqual(parseApiVersion("g/v"), { group: "g", version: "v" });
  assert.equal(parseApiVersion(""), undefined);
  assert.equal(catalog.get("external-secrets.io/ExternalSecret")?.version, "v1");
});

test("import: catalog kinds editable, other versions and kinds read-only, refusals reported", () => {
  const { project, issues } = load(`${RECIPE}
---
apiVersion: v1
kind: ConfigMap
metadata: { name: cm }
---
apiVersion: external-secrets.io/v1beta1
kind: ExternalSecret
metadata: { name: old }
---
apiVersion: external-secrets.io/v0
kind: ExternalSecret
metadata: { name: older }
---
kind: NoApiVersion
---
`);
  assert.deepEqual(
    project.resources.map((r) => [nameOf(r.doc), r.mode]),
    [
      ["docs", "editable"],
      ["main", "editable"],
      ["example-token", "editable"],
      ["cm", "unknown"],
      ["old", "readOnly"],
      ["older", "readOnly"],
    ],
  );
  assert.deepEqual(codes(issues), ["invalid-document", "other-version", "unknown-kind", "unsupported-version"]);
  assert.equal(issues.find((i) => i.code === "other-version")?.resource, "r5");
  assert.deepEqual(codes(load("a: [").issues), ["invalid-document"]);
});

test("graph: references resolve by kind, name and namespace; made and external objects are nodes", () => {
  let { project } = load(RECIPE);
  ({ project } = addExternal(project, { kind: secret, name: "admin", keys: ["password"] }));
  const graph = buildGraph(project, catalog);
  assert.deepEqual(
    graph.nodes.map((n) => [n.id, n.kind.kind, n.name, n.namespace, n.origin]),
    [
      ["r1", "ExampleObject", "docs", "team-a", "resource"],
      ["r2", "ExampleLocalInstance", "main", "team-a", "resource"],
      ["r3", "ExternalSecret", "example-token", "team-a", "resource"],
      ["r3/produces/0/0", "Secret", "example-token", "team-a", "produced"],
      ["x4", "Secret", "admin", "team-a", "external"],
    ],
  );
  assert.deepEqual(graph.nodes[3].keys, ["token"]);
  assert.deepEqual(
    graph.slots.map((s) => [s.resource, formatPath(s.path), s.target?.id]),
    [
      ["r1", "spec.instanceRef", "r2"],
      ["r2", "spec.tokenSecretRef", "r3/produces/0/0"],
    ],
  );
  assert.deepEqual(
    candidates(graph, graph.slots[1]).map((n) => n.id),
    ["r3/produces/0/0", "x4"],
  );
  assert.deepEqual(readRef("n"), { name: "n" });
  assert.equal(readRef(""), undefined);
  assert.equal(readRef({ name: 1 }), undefined);
  assert.deepEqual(readRef({ name: "n", namespace: "ns", key: "k" }), { name: "n", namespace: "ns", key: "k" });
});

test("validate: a complete project has no issue", () => {
  const { project } = load(RECIPE);
  assert.deepEqual(validate(project, catalog), []);
});

test("validate: the schema's rules, holes and names", () => {
  const { project } = load(`
apiVersion: ${G}/v1alpha1
kind: ExampleObject
metadata: { name: Bad_Name }
spec:
  instanceRef: null
  name: TOOLONGNAME
  replicas: 9
  tags: [a, 1]
  labels: { a: 1 }
  extra: { anything: [1] }
  nope: true
---
apiVersion: ${G}/v1alpha1
kind: ExampleInstance
metadata: { name: "<instance name>" }
spec:
  url: "<url>"
  tokenSecretRef: { name: t, namespace: team-a }
`);
  const issues = validate(project, catalog);
  assert.deepEqual(
    issues.map((i) => [i.resource, i.code, i.path]),
    [
      ["r1", "invalid-name", "metadata.name"],
      ["r1", "pattern", "spec.name"],
      ["r1", "range", "spec.name"],
      ["r1", "range", "spec.replicas"],
      ["r1", "range", "spec.tags"],
      ["r1", "type", "spec.tags[1]"],
      ["r1", "type", "spec.labels.a"],
      ["r1", "unknown-field", "spec.nope"],
      ["r2", "hole", "metadata.name"],
      ["r2", "missing-required", "spec.tokenSecretRef.key"],
      ["r2", "hole", "spec.url"],
      ["r2", "dangling-reference", "spec.tokenSecretRef"],
    ],
  );
  assert.ok(isHole("<x>") && !isHole("x") && !isHole(1));
  assert.match(issues[0].message, /"Bad_Name" is not a valid name/);
});

test("validate: references — wrong kind, missing key, namespace, sharing, duplicates", () => {
  const { project } = load(`
apiVersion: ${G}/v1alpha1
kind: ExampleObject
metadata: { name: wrong, namespace: team-a }
spec: { instanceRef: { name: example-token } }
---
apiVersion: ${G}/v1alpha1
kind: ExampleLocalInstance
metadata: { name: main, namespace: team-b }
spec:
  tokenSecretRef: { name: example-token, key: nope }
---
apiVersion: ${G}/v1alpha1
kind: ExampleObject
metadata: { name: far, namespace: team-a }
spec: { instanceRef: { name: main, namespace: team-b } }
---
apiVersion: ${G}/v1alpha1
kind: ExampleInstance
metadata: { name: cluster }
spec:
  url: u
  tokenSecretRef: { name: s, key: k }
---
apiVersion: ${G}/v1alpha1
kind: ExampleInstance
metadata: { name: cluster }
spec:
  url: u
  tokenSecretRef: { name: example-token, namespace: team-b, key: token }
---
apiVersion: external-secrets.io/v1
kind: ExternalSecret
metadata: { name: example-token, namespace: team-b }
spec:
  data: [{ secretKey: token }]
`);
  const issues = validate(project, catalog);
  assert.deepEqual(
    issues.map((i) => [i.resource, i.code]),
    [
      ["r4", "missing-required"],
      ["r1", "dangling-reference"],
      ["r2", "missing-key"],
      ["r3", "not-shared"],
      ["r4", "missing-namespace"],
      ["r5", "duplicate"],
    ],
  );
  assert.match(issues[2].message, /has no key nope \(known: token\)/);
  assert.match(issues[3].message, /team-b\/main is not shared with namespace team-a/);

  // The same name, another kind, same namespace: a wrong kind.
  const wrong = load(`
apiVersion: ${G}/v1alpha1
kind: ExampleObject
metadata: { name: wrong, namespace: team-a }
spec: { instanceRef: { name: example-token } }
---
apiVersion: external-secrets.io/v1
kind: ExternalSecret
metadata: { name: example-token, namespace: team-a }
`).project;
  assert.deepEqual(codes(validate(wrong, catalog)), ["wrong-kind"]);

  // Shared: by namespace, by selector, with cluster-scoped objects.
  const sharing = (sharedWith, from = "team-a") =>
    codes(
      validate(
        load(`
apiVersion: ${G}/v1alpha1
kind: ExampleLocalInstance
metadata: { name: main, namespace: team-b }
spec: { tokenSecretRef: { name: s, key: k }, sharedWith: ${JSON.stringify(sharedWith)} }
---
apiVersion: v1
kind: Secret
metadata: { name: s, namespace: team-b }
---
${
  from
    ? `apiVersion: ${G}/v1alpha1
kind: ExampleObject
metadata: { name: o, namespace: ${from} }
spec: { instanceRef: { name: main, namespace: team-b } }`
    : `apiVersion: ${G}/v1alpha1
kind: ExampleInstance
metadata: { name: c }
spec: { url: u, tokenSecretRef: { name: s, namespace: team-b, key: k } }`
}
`).project,
        catalog,
      ),
    );
  assert.deepEqual(sharing({ namespaces: ["team-a"] }), []);
  assert.deepEqual(sharing({ namespaceSelector: {} }), []);
  assert.deepEqual(sharing({}), ["not-shared"]);
});

test("export: dependency order, the project's namespace, empty fields pruned", () => {
  let { project } = load(RECIPE);
  project = setDefaultNamespace(project, "team-a");
  ({ project } = addResource(project, { ...newDoc(catalog, instance, "cluster"), spec: { url: "u", tokenSecretRef: { name: "s", namespace: "n", key: "k" } } }));
  assert.deepEqual(
    dependencyOrder(project, catalog).map((r) => nameOf(r.doc)),
    ["example-token", "main", "docs", "cluster"],
  );
  const out = exportYaml(project, catalog);
  const docs = yaml.loadAll(out);
  assert.equal(docs.length, 4);
  assert.deepEqual(docs[3].metadata, { name: "cluster" }, "a cluster-scoped kind has no namespace");
  assert.equal(docs[2].spec.replicas, undefined);
  // An enum `kind` beside a name is a field, not a union: the reference stays.
  assert.deepEqual(docs[2].spec.instanceRef, { name: "main" });

  // The YAML is the project: imported again, it gives the same documents.
  const again = load(out).project;
  assert.equal(exportYaml(again, catalog), out);

  assert.deepEqual(
    exportFiles(project, catalog).map((f) => f.name),
    [
      "01-externalsecret-team-a-example-token.yaml",
      "02-examplelocalinstance-team-a-main.yaml",
      "03-exampleobject-team-a-docs.yaml",
      "04-exampleinstance-cluster.yaml",
    ],
  );

  // A namespaced resource without a namespace takes the project's.
  const bare = addResource(emptyProject("ns"), newDoc(catalog, local, "l")).project;
  assert.equal(yaml.load(exportYaml(bare, catalog)).metadata.namespace, "ns");
  assert.throws(() => newDoc(catalog, { group: "x", kind: "Y" }, "n"), /not in the catalog/);
});

test("export: a cycle does not lose a resource", () => {
  const cyclic = {
    ...links,
    kinds: { ...links.kinds, ExampleLocalInstance: { references: [{ path: "spec.tokenSecretRef", to: [local] }], produces: [] } },
  };
  const c = buildCatalog([{ kinds: schemas, schemas, links: cyclic }]);
  const { project } = importYaml(
    emptyProject("a"),
    c,
    `
apiVersion: ${G}/v1alpha1
kind: ExampleLocalInstance
metadata: { name: x }
spec: { tokenSecretRef: { name: y, key: k } }
---
apiVersion: ${G}/v1alpha1
kind: ExampleLocalInstance
metadata: { name: y }
spec: { tokenSecretRef: { name: x, key: k } }
`,
  );
  assert.deepEqual(dependencyOrder(project, c).map((r) => nameOf(r.doc)), ["x", "y"]);
  const positions = layout(buildGraph(project, c));
  assert.equal(positions.size, 2);
});

test("edits: read-only resources stay as imported, removals, externals", () => {
  let { project } = load(`${RECIPE}
---
apiVersion: external-secrets.io/v1beta1
kind: ExternalSecret
metadata: { name: old }
`);
  const before = project.resources[3].doc;
  project = updateResource(project, "r4", { changed: true });
  assert.equal(project.resources[3].doc, before);
  project = updateResource(project, "r1", { ...project.resources[0].doc, spec: { name: "new" } });
  assert.deepEqual(project.resources[0].doc.spec, { name: "new" });
  project = removeResource(project, "r4");
  assert.equal(project.resources.length, 3);
  const added = addExternal(project, { kind: secret, name: "s", namespace: "other", keys: [] });
  assert.equal(added.id, "x4");
  assert.equal(removeExternal(added.project, "x4").externals.length, 0);
});

test("layout: a column per dependency depth", () => {
  const { project } = load(RECIPE);
  const positions = layout(buildGraph(project, catalog), { x: 10, y: 1 });
  assert.deepEqual(Object.fromEntries(positions), {
    r1: { x: 30, y: 0.5 },
    r2: { x: 20, y: 0 },
    r3: { x: 0, y: 0 },
    "r3/produces/0/0": { x: 10, y: 0.5 },
  });
});

test("zip: stored entries any unzip reads", () => {
  assert.equal(crc32(new TextEncoder().encode("123456789")), 0xcbf43926);
  const bytes = zip([
    { name: "01-a.yaml", content: "a: 1\n" },
    { name: "02-é.yaml", content: "b: 2\n" },
  ]);
  const view = new DataView(bytes.buffer);
  assert.equal(view.getUint32(0, true), 0x04034b50);
  const end = bytes.length - 22;
  assert.equal(view.getUint32(end, true), 0x06054b50);
  assert.equal(view.getUint16(end + 10, true), 2);
  const centralOffset = view.getUint32(end + 16, true);
  assert.equal(view.getUint32(centralOffset, true), 0x02014b50);
  const nameLength = view.getUint16(centralOffset + 28, true);
  assert.equal(new TextDecoder().decode(bytes.slice(centralOffset + 46, centralOffset + 46 + nameLength)), "01-a.yaml");
});

test("slots: empty ones offered, pointing writes the name and only a foreign namespace", () => {
  let { project } = load(`
apiVersion: ${G}/v1alpha1
kind: ExampleObject
metadata: { name: o, namespace: team-a }
spec: {}
---
apiVersion: ${G}/v1alpha1
kind: ExampleLocalInstance
metadata: { name: here, namespace: team-a }
spec: { tokenSecretRef: { name: s, key: token } }
---
apiVersion: ${G}/v1alpha1
kind: ExampleLocalInstance
metadata: { name: there, namespace: team-b }
spec: { tokenSecretRef: { name: s, key: token } }
`);
  let graph = buildGraph(project, catalog);
  const [slot] = emptySlots(graph, project, catalog, "r1");
  assert.equal(formatPath(slot.path), "spec.instanceRef");
  assert.deepEqual(emptySlots(graph, project, catalog, "r2"), [], "a filled slot is not offered again");
  assert.deepEqual(emptySlots(graph, project, catalog, "nope"), []);

  const there = graph.nodes.find((n) => n.name === "there");
  let doc = pointAt(project, catalog, slot, there);
  assert.deepEqual(doc.spec.instanceRef, { name: "there", namespace: "team-b" });
  project = updateResource(project, "r1", doc);
  graph = buildGraph(project, catalog);
  const filled = graph.slots.find((s) => s.resource === "r1");
  doc = pointAt(project, catalog, filled, graph.nodes.find((n) => n.name === "here"));
  assert.deepEqual(doc.spec.instanceRef, { name: "here" });
  assert.equal(pointAt(project, catalog, filled, undefined).spec.instanceRef, undefined);
  assert.equal(pointAt(project, catalog, { ...filled, resource: "nope" }, undefined), undefined);

  // The key a SecretKeyRef holds stays; a bare-name field takes the name.
  const secretSlot = graph.slots.find((s) => s.resource === "r2");
  const other = { id: "x", kind: secret, name: "other", namespace: "team-a", origin: "external" };
  assert.deepEqual(pointAt(project, catalog, secretSlot, other).spec.tokenSecretRef, { name: "other", key: "token" });
  const bare = { ...project, resources: [{ id: "b", mode: "editable", doc: { spec: { ref: "old" } } }] };
  assert.deepEqual(pointAt(bare, catalog, { ...secretSlot, resource: "b", path: ["spec", "ref"] }, other).spec, { ref: "other" });
});

test("produces: one object per item, namespaced by its field, a skipped backend, fixed keys", () => {
  const targets = {
    ...links,
    kinds: {
      ...links.kinds,
      ExampleObject: {
        references: links.kinds.ExampleObject.references,
        produces: [
          {
            kind: secret,
            each: "spec.tags[]",
            name: ["name"],
            namespace: "namespace",
            keyNames: ["token"],
            unless: { path: "backend", equals: "vault" },
          },
        ],
      },
    },
  };
  const c = buildCatalog([{ kinds: schemas, schemas, links: targets }]);
  const { project } = importYaml(
    emptyProject("team-a"),
    c,
    `
apiVersion: ${G}/v1alpha1
kind: ExampleObject
metadata: { name: o }
spec:
  tags:
    - { name: here }
    - { name: there, namespace: team-b }
    - { name: in-vault, backend: vault }
    - { namespace: team-b }
`,
  );
  const made = buildGraph(project, c).nodes.filter((n) => n.origin === "produced");
  assert.deepEqual(
    made.map((n) => [n.id, n.name, n.namespace, n.keys]),
    [
      ["r1/produces/0/0", "here", "team-a", ["token"]],
      ["r1/produces/0/1", "there", "team-b", ["token"]],
    ],
  );
});

test("lazy schemas: listed kinds, specs unchecked and unpruned until their schema is loaded", () => {
  const lazy = buildCatalog([{ kinds: schemas, links }, { kinds: externalSchemas, links: external.links }]);
  const { project } = load(RECIPE, emptyProject("team-a"));
  const loaded = importYaml(emptyProject("team-a"), lazy, RECIPE).project;
  assert.deepEqual(missingSchemas(lazy, loaded.resources.map((r) => groupKindOf(r.doc))), [
    `${G}/ExampleObject`,
    `${G}/ExampleLocalInstance`,
    "external-secrets.io/ExternalSecret",
  ]);
  const bad = updateResource(loaded, "r1", { ...loaded.resources[0].doc, spec: { replicas: 99, instanceRef: { name: "main" } } });
  assert.deepEqual(codes(validate(bad, lazy)), [], "no schema: nothing checked");
  assert.match(exportYaml(bad, lazy), /replicas: 99/);
  const full = withSchemas(lazy, [...schemas, ...externalSchemas]);
  assert.deepEqual(missingSchemas(full, loaded.resources.map((r) => groupKindOf(r.doc))), []);
  assert.deepEqual(codes(validate(bad, full)), ["range"]);
  assert.equal(withSchemas(lazy, [{ ...schemas[0], kind: "Nope" }]).get(`${G}/Nope`), undefined);
  assert.ok(project.resources.length === 3);
});
