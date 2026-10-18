# weebo-operator-kit

Shared core of the weebo Kubernetes operators (weebo-forgejo,
weebo-authentik, future ones). **`docs/plan.md`** is the authoritative
design doc (what belongs in the kit, versioning, migration roadmap) —
read it before any non-trivial change.

## Rules

- Hexagonal layering, same as the operators: `weebo-kit-domain` has no
  dependency; `weebo-kit-api` only serde/schemars/k8s-openapi;
  `weebo-kit-application` has no adapter or Kubernetes client;
  adapters/runtime/testkit hold the `kube`/Vault/otel code.
- Nothing in the kit names a product (Forgejo, Authentik…) or a concrete
  CRD. What varies between operators is a parameter or a generic type.
- Doc comments of `weebo-kit-api` types become CRD schema descriptions in
  every operator: talk about "the instance" / "the remote object".
- No reason code is defined here: generic code goes through the `Reason`
  trait, errors that need a specific code stay neutral and the operator
  maps them.
- `kube`/`k8s-openapi` versions must match the operators'.
- A change that breaks an operator's build is a breaking release (0.x:
  minor bump).
- `packages/operator-docs` (`@weebo/operator-docs`, npm) is the operators'
  docs-site format: CRD page generator, manifest builder, theme, layout,
  routes. Same naming rule (what varies is `defineSite`, the generator's
  config or `CrdChoices`); its version is the workspace's
  (`.hooks/bump-versions.sh`); its `fumadocs-*`/`next` peers are pinned,
  moving them is a release. Its reference pages (reason codes, Events,
  metrics, budget) quote kit values from copies in `src/reference/*.json`:
  changing a runtime constant or `Budget::small_cluster()` means updating
  them (tests in `weebo-kit-runtime`/`weebo-kit-perf` fail otherwise). See
  its README.

- `charts/weebo-operator-lib`, `tasks/*.yaml`, `.github/workflows/checks.yml`,
  `.github/workflows/publish.yml` and `templates/operator` are consumed by tag (OCI chart, raw URLs pinned
  by sha256, `uses: …@vX.Y.Z`, `cargo generate --tag`): same naming rule,
  and a change that breaks an operator is a breaking release. The chart
  keeps weebo-forgejo's resource names and values contract. The template
  is checked by `task template:check` (in `test:all`); its pins on the kit
  are rewritten by `.hooks/bump-versions.sh` (`templates/pin-kit.sh`).
  Never render the template inside the kit's tree with cargo-generate
  directly: it adds the project to the kit's workspace and rewrites
  `Cargo.toml` (`task template:render` renders outside, then moves).

## Commands

```bash
task lint    # fmt --check, clippy -D warnings, actionlint, chart:check
task test    # unit + contract + doctests
task test:all       # + template:check (the operator template, built and tested)
task template:render  # the template rendered for "Example" in target/template/
task audit   # cargo-deny + trivy + npm audit (operator-docs)
task doctor  # envtest prerequisites
task docs:check  # operator-docs: version sync, tsc, generator tests
task docs:pack   # dist/weebo-operator-docs-<version>.tgz (release asset)
```

Commits follow Conventional Commits (cocogitto). `cog.toml` is rendered
from `templates/cog.toml` (`task kit:cog`), the same template every
operator renders (`templates/kit.yaml`, copied into its task directory):
edit the template, never a rendered `cog.toml`. Its hooks: `cog verify`
on every message; `recu`, `lint`, `test`, `audit` before every commit;
`audit` and `test:all` (or `test`) before every tag.

Every rule of the user's global Weebo Dev Env guideline (Taskfile with
`desc`, mise, signed commits by the user only, `task audit`) applies.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

Rules:
- When graphify-out/graph.json exists, run `graphify query "<question>"` before your first grep, search or source-file read for a codebase question, and put this rule in every subagent prompt that explores code. Read raw files only for lines the graph or an earlier read already located. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
- A committed graphify-out/graph.json is rewritten whole on every build, so a large diff to it in a pull request is expected. Do not flag it as churn or suggest skipping the graph rebuild or its hooks.

<!-- rtk-instructions v2 -->
# Command output

Command output here is condensed to save tokens, keeping every signal and
dropping costly noise. Treat it as the complete result: run commands
normally, and batch related commands into one call to avoid extra turns.
Truncated results state their recovery path in their own output. Re-run a
command as `rtk bypass <cmd>` only when its result is unusable: empty when
output was clearly expected, contradicting its exit code, or garbled.
<!-- /rtk-instructions -->
