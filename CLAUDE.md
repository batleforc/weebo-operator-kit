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

## Commands

```bash
task lint    # fmt --check, clippy -D warnings, actionlint
task test    # unit + contract + doctests
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
