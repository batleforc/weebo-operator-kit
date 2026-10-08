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

## Commands

```bash
task lint    # fmt --check, clippy -D warnings, actionlint
task test    # unit + contract + doctests
task audit   # cargo-deny + trivy
task doctor  # envtest prerequisites
```

Commits follow Conventional Commits (cocogitto, `cog.toml`).

Every rule of the user's global Weebo Dev Env guideline (Taskfile with
`desc`, mise, signed commits by the user only, `task audit`) applies.
