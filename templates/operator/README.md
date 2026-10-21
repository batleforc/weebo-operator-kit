# {{project-name}}

Kubernetes operator managing {{product}} objects as CRDs, built on
[weebo-operator-kit](https://github.com/batleforc/weebo-operator-kit).

| Kind                | Scope      | What it does                                                       |
| ------------------- | ---------- | ------------------------------------------------------------------ |
| `{{product}}Instance` | Cluster    | A {{product}} to drive: its URL and API token (`tokenSecretRef`) |
| `{{product}}Object`   | Namespaced | An object of that {{product}}, created, updated and deleted with the CR |

## Layout

Hexagonal, one crate per layer, same rules as every weebo operator:

| Crate               | Holds                                                                    |
| ------------------- | ------------------------------------------------------------------------ |
| `domain`            | the reason codes (`reason_codes!`), no dependency                        |
| `api`               | the CRD types (`#[derive(CustomResource)]`), the kit's shared field types |
| `application`       | ports (`RemoteGateway`, `GatewayFactory`) and the `reconcile_*` use cases |
| `adapters-outbound` | the {{product}} HTTP gateway — **the placeholder to replace with the real client** — and its factory |
| `adapters-inbound`  | the controllers and the webhook router (health routes only for now)       |
| `operator`          | the binary: telemetry, leader election, webhook server, controllers      |
| `crdgen`            | writes `deploy/crd/` from `api` (CRDs, and `links.json` for the CRD IDE)  |
| `testkit`           | envtest and fakes for the integration tests                               |

`deploy/charts/{{project-name}}` is a thin chart on the kit's library
chart: `templates/_params.tpl` holds what is the operator's own (group,
kinds, env, egress), `values.yaml` follows the kit's values contract.

## First steps

The project is licensed `{{license}}` (`Cargo.toml`, the image's labels):
add the matching `LICENSE.md`.

```bash
task init     # toolchain (mise) and mise.lock — commit it, cog.toml from the kit, git hooks
task doctor   # envtest prerequisites
task recu     # CRDs (crdgen) and their copies in the chart
task test:all # unit, contract and envtest tests — creates Cargo.lock: commit it
```

Then replace the placeholder REST shape of
`crates/adapters-outbound/src/http_gateway.rs` with the real
{{product}} API, and grow `{{product}}Object` into the real kinds.

## Tasks

```bash
task lint        # fmt --check, clippy -D warnings, actionlint, hadolint
task lint:helm   # helm lint + the kit's smoke renders
task test        # unit + contract tests
task test:all    # + envtest integration tests
task coverage    # cargo llvm-cov, fails under 85%
task audit       # cargo-deny + trivy + trivy config
task kit:sync TAG=vX.Y.Z  # move to another kit release (crates, tasks, CI, chart, cog.toml)
```

`lint`, `audit`, `coverage`, `doctor` and the chart tasks are the kit's
(`tasks/*.yaml`, included remotely and pinned by checksum); CI calls the
kit's reusable `checks.yml`.
