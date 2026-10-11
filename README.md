# weebo-operator-kit

The pipe dream of this project is to easen the creation of kubernetes operators in Rust, and in the end cover all the tool i may need to build / maintain / document them.

In addition, i will use them in my project [Weebo-SI](https://github.com/batleforc/weebo-si) and in a futur project :eyes:.

Shared core of the weebo Kubernetes operators
([weebo-forgejo](https://github.com/batleforc/weebo-forgejo),
[weebo-authentik](https://github.com/batleforc/weebo-authentik), and the
next ones): the plumbing every operator needs around its own business
logic, in the same hexagonal layout the operators use.

| Crate                   | What it gives an operator                                                                                                 |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `weebo-kit-domain`      | `reason_codes!` + `Reason`, `Failure<R>`/`Advisory<R>`, the namespace allow-list engine                                   |
| `weebo-kit-api`         | CRD field types: `Condition`, `SecretKeyRef`, `ObjectRef`, `SecretTarget`, `TlsOptions`; the common annotations          |
| `weebo-kit-application` | `SecretStore`/`PolicySource` ports, secret-emission helpers, `*Ref` resolution, `FakeSecretStore` (feature `test-support`) |
| `weebo-kit-adapters`    | Kubernetes and Vault secret stores, per-target routing, instance resolution, the importer's adoption flow                 |
| `weebo-kit-runtime`     | telemetry, leader election, health, shutdown, TLS webhook server, `/validate`, controller skeleton, status, Events       |
| `weebo-kit-testkit`     | envtest control plane, status polling                                                                                     |
| `weebo-kit-perf`        | what the operator costs: its release binary against envtest, RSS/CPU/fds/threads, API traffic, convergence, a budget     |

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
