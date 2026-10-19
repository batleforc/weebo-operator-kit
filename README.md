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
| `templates/{cog.toml,kit.yaml,pin-kit.sh}`                 | `cog.toml`, `task kit:cog` and `task kit:sync TAG=vX.Y.Z` (every pin on a kit release; a repository without a kit crate pins `KIT_TAG` in its Taskfile, a pnpm docs site sets `NPM_CLIENT: pnpm`) |

Design, scope rules, versioning and the migration roadmap:
[`docs/plan.md`](docs/plan.md).

**Documentation site: <https://batleforc.github.io/weebo-operator-kit/>**
— how the kit works (crates, library chart, tooling, guides) and which
operators use it. Its sources are in [`site/`](site/), built on
`@weebo/operator-docs` from this tree (`file:../packages/operator-docs`),
and published to GitHub Pages by `.github/workflows/site-pages.yml` on
every push to `main` touching `site/` or `packages/operator-docs/`.

## Using it

```toml
# operator's Cargo.toml
[workspace.dependencies]
weebo-kit-domain = { git = "https://github.com/batleforc/weebo-operator-kit", tag = "v0.1.0" }
weebo-kit-runtime = { git = "https://github.com/batleforc/weebo-operator-kit", tag = "v0.1.0" }
# …one line per crate used
```

Keep `kube` and `k8s-openapi` on the kit's versions.

### Kubernetes version

The kit's libraries enable no `v1_*` feature of `k8s-openapi`: the operator
picks its Kubernetes version, as `k8s-openapi` requires of an application.
Enable the feature in the operator's workspace, at the oldest apiserver it
supports (the template uses `v1_36`):

```toml
k8s-openapi = { version = "0.28", features = ["v1_36", "schemars"] }
```

The kit is tested against 1.36 and compiles for 1.32 to 1.36, each version
checked in CI (`k8s-versions` job). RFC 0005 (`docs/rfc-0005-k8s-openapi-version.md`).

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
task audit    # cargo-deny + trivy + npm audit (operator-docs, site)
task doc      # rustdoc
task docs:check  # operator-docs: version sync, tsc, generator tests
task docs:pack   # dist/weebo-operator-docs-<version>.tgz
task site        # the documentation site (site/) on http://localhost:3000
task site:build  # lint + static export into site/out/
```

`task site:deps` reinstalls the site's dependencies whenever
`packages/operator-docs` changes (the package is installed as a copy);
restart `task site` to see a package change.

To iterate on the kit and an operator together, point the operator at a
sibling checkout with `path = "../weebo-operator-kit/crates/<crate>"` —
never push an operator with a path dependency.
