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

Plus one npm package for the operators' documentation sites:

| Package                                                  | What it gives an operator's docs site                                                        |
| -------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| [`@weebo/operator-docs`](packages/operator-docs/README.md) | CRD reference generator (`weebo-crd-docs`), manifest builder, theme, layouts, routes |

And what the operators kept copying around the code:

| Path                                                       | What it gives an operator                                                                  |
| ---------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| [`charts/weebo-operator-lib`](charts/weebo-operator-lib/README.md) | Library chart (`oci://ghcr.io/batleforc/charts`): every chart resource but the CRDs, from one parameter set |
| `tasks/{rust,helm,repo}.yaml`                              | Shared Taskfiles, included remotely and pinned by checksum: lint, audit, coverage, doctor, chart lint/CRDs |
| `.github/workflows/checks.yml`                             | Reusable CI: lint, test, audit, chart lint (`perf.yml`: the weekly resource budget)        |
| `templates/operator`                                       | `cargo-generate` template of a new operator, wired on all of the above                     |
| `templates/{cog.toml,kit.yaml,pin-kit.sh}`                 | `cog.toml`, `task kit:cog` and `task kit:sync TAG=vX.Y.Z` (every pin on a kit release)     |

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

A new operator starts from the template:

```bash
cargo generate --git https://github.com/batleforc/weebo-operator-kit --tag vX.Y.Z templates/operator
```

```json
// operator's docs/package.json: the tarball attached to the kit's release
"@weebo/operator-docs": "https://github.com/batleforc/weebo-operator-kit/releases/download/vX.Y.Z/weebo-operator-docs-X.Y.Z.tgz"
```

## Development

```bash
task init     # toolchain (mise) + git hooks (cocogitto)
task doctor   # envtest prerequisites (Go, libclang, etcd, kube-apiserver)
task lint     # fmt --check, clippy -D warnings, actionlint, library chart (chart:check)
task test     # unit + contract tests + doctests
task test:all # + the operator template rendered, built and tested (template:check)
task audit    # cargo-deny + trivy + npm audit (operator-docs)
task doc      # rustdoc
task docs:check  # operator-docs: version sync, tsc, generator tests
task docs:pack   # dist/weebo-operator-docs-<version>.tgz
```

To iterate on the kit and an operator together, point the operator at a
sibling checkout with `path = "../weebo-operator-kit/crates/<crate>"` —
never push an operator with a path dependency.
