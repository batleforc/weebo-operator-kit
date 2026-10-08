# weebo-operator-kit

Shared core of the weebo Kubernetes operators
([weebo-forgejo](https://github.com/batleforc/weebo-forgejo),
[weebo-authentik](https://github.com/batleforc/weebo-authentik), and the
next ones): the plumbing every operator needs around its own business
logic, in the same hexagonal layout the operators use.

| Crate                   | What it gives an operator                                                                 |
|-------------------------|-------------------------------------------------------------------------------------------|
| `weebo-kit-domain`      | `reason_codes!` + `Reason`, `Failure<R>`/`Advisory<R>`, the namespace allow-list engine   |
| `weebo-kit-api`         | CRD field types: `Condition`, `SecretKeyRef`, `ObjectRef`, `SecretTarget`, `TlsOptions`   |
| `weebo-kit-application` | `SecretStore` ports, secret-emission helpers, `FakeSecretStore` (feature `test-support`) |
| `weebo-kit-adapters`    | Kubernetes and Vault (Kubernetes auth, KV v2) secret stores, per-target routing           |
| `weebo-kit-runtime`     | telemetry, leader election, graceful shutdown, TLS webhook server, metrics, conditions    |
| `weebo-kit-testkit`     | envtest control plane, status polling                                                     |

Design, scope rules, versioning and the migration roadmap:
[`docs/plan.md`](docs/plan.md).

## Using it

```toml
# operator's Cargo.toml
[workspace.dependencies]
weebo-kit-domain = { git = "https://github.com/batleforc/weebo-operator-kit", tag = "v0.1.0" }
weebo-kit-runtime = { git = "https://github.com/batleforc/weebo-operator-kit", tag = "v0.1.0" }
# …one line per crate used
```

Keep `kube` and `k8s-openapi` on the kit's versions.

## Development

```bash
task init     # toolchain (mise) + git hooks (cocogitto)
task doctor   # envtest prerequisites (Go, libclang, etcd, kube-apiserver)
task lint     # fmt --check, clippy -D warnings, actionlint
task test     # unit + contract tests + doctests
task audit    # cargo-deny + trivy
task doc      # rustdoc
```

To iterate on the kit and an operator together, point the operator at a
sibling checkout with `path = "../weebo-operator-kit/crates/<crate>"` —
never push an operator with a path dependency.
