# weebo-operator-lib

Library chart of the weebo operators: every resource an operator chart
deploys except its CRDs — Deployment, ServiceAccount and RBAC, leader
election Role, admission webhook (Service, cert-manager Issuer and
Certificate, `ValidatingWebhookConfiguration`), PodDisruptionBudget,
metrics Service, ServiceMonitor, PrometheusRule, NetworkPolicy, and the
Argo CD health check. Published to `oci://ghcr.io/batleforc/charts` at
every kit release, under the kit's version.

Nothing here names a product: names come from the operator chart's own
`Chart.yaml` (`.Chart.Name`), what varies is a parameter set.

## An operator chart

```yaml
# Chart.yaml
dependencies:
  - name: weebo-operator-lib
    version: X.Y.Z # the kit's, as in Cargo.toml (task kit:sync)
    repository: oci://ghcr.io/batleforc/charts
```

```yaml
# templates/operator.yaml — the whole chart, CRDs aside
{{ include "weebo-operator.all" (dict "ctx" . "params" (include "operator.params" . | fromYaml)) }}
```

```yaml
# templates/_params.tpl
{{- define "operator.params" -}}
group: example.weebo.io        # the CRDs' API group
versions: [v1alpha1]           # admitted versions (webhook rules)
kinds:                         # every kind the operator watches (ClusterRole)
  - exampleinstances
  - exampleobjects
namespacedKinds:               # the namespaced kinds: what a team may manage
                               # (rbac.namespacedExample), the watch Role (watch.namespace)
  - exampleobjects
admitted:                      # kinds sent to /validate; empty: no webhook configuration
  - exampleobjects
env:                           # the operator's own environment variables
  - name: WEEBO_EXAMPLE_MAX_INFLIGHT
    value: {{ .Values.exampleMaxInflight | quote }}
egress:                        # NetworkPolicy egress to the remote system
  - comment: Example instances.
    rules: {{ toJson .Values.networkPolicy.egress.example }}
clusterRules: []               # extra ClusterRole rules
watchRules: []                 # extra rules of the watch Role (watch.namespace):
                               # the namespaced resources the operator reads or
                               # writes in the watched namespace
clusterSecrets: true           # false: with watch.namespace, no Secret access
                               # beyond that namespace (no cluster-scoped kind
                               # reads or writes Secrets); default true
alerts: []                     # extra PrometheusRule rules, after the kit's
{{- end -}}
```

`NOTES.txt` can start with `{{ include "weebo-operator.notes" . }}` (what
is installed, prerequisites, warnings) and add the operator's first steps.

The CRDs ship as templates under `crds.install`, with the chart's labels
and `helm.sh/resource-policy: keep`: the kit's `helm:crds` task
(`tasks/helm.yaml`) wraps crdgen's output with
`weebo-operator.crdMetadata`.

## Values

The values contract is the one of `templates/operator`'s chart
(`deploy/charts/<operator>/values.yaml` and its `values.schema.json`):
`replicaCount`, `image`, `serviceAccount`, `crds`, `reconcileConcurrency`,
`workerThreads` (optional: unset, the binary's default applies), `logFormat`, `watch`, `leaderElection`, `webhook`, `certManager`, `metrics`,
`serviceMonitor`, `prometheusRule`, `probes`, `shutdown`,
`podDisruptionBudget`, `rbac.namespacedExample`, `resources`, scheduling,
security contexts, `networkPolicy` and `extraEnv`. An operator adds its
own keys next to them (and `weebo-operator-lib: {}`, which Helm sets for
the dependency, to its schema).

Kept as they were in weebo-forgejo's chart: resource names
(`<fullname>`, `-webhook`, `-webhook-tls`, `-leader-election`, `-metrics`,
`-argocd-health`, `-author`), the Lease name (`<chart>-operator-leader`),
the webhook name (`validate.<group>`).

## Watch scope

`watch.namespace` and `watch.labelSelector` (a `LabelSelector`) reach the
operator as `WEEBO_WATCH_NAMESPACE` and `WEEBO_WATCH_LABEL_SELECTOR` (the
kit's `WatchScope`). With a namespace, the namespaced kinds leave the
ClusterRole for a Role in that namespace. With a selector, the
`scope.<group>` webhook sends every UPDATE to the operator's `/scope` (the
kit's `ScopeGuard`). The webhooks' `namespaceSelector`/`objectSelector`
default to the scope. Nothing set: the whole cluster, as before.

`params.clusterRules` only reach the ClusterRole, which a release with
`watch.clusterKinds: false` does not render. The rules an operator needs
on namespaced resources of the watched namespace (the Deployments,
Services, NetworkPolicies… it writes for a namespaced CR) go to
`params.watchRules` too: rendered in the watch Role (`<fullname>-watch`)
whenever `watch.namespace` is set, so a full-namespaced release keeps
them, limited to that namespace. Typically:

```yaml
{{- $namespaced := list (dict "apiGroups" (list "apps") "resources" (list "deployments") "verbs" (list "get" "create" "patch" "delete")) }}
clusterRules: {{ toJson $namespaced }}
watchRules: {{ toJson $namespaced }}
```

## The operator's own pod

`WEEBO_OPERATOR_POD_LABELS` holds the operator pod's selector labels as a
JSON object (`{"app.kubernetes.io/instance":"…","app.kubernetes.io/name":"…"}`,
`nameOverride` applied — the Deployment's `matchLabels`), next to
`POD_NAMESPACE`. An operator writing NetworkPolicies for what it deploys
uses them to let itself in (`podSelector.matchLabels` +
`namespaceSelector` on `kubernetes.io/metadata.name`); the kit reads them
with `weebo_kit_runtime::pod::own_labels`.

## Templates

| Template                                   | Renders                                                      |
| ------------------------------------------ | ------------------------------------------------------------ |
| `weebo-operator.all`                       | everything below, `---`-separated, skipping what is disabled  |
| `weebo-operator.deployment`                | the operator Deployment (probes, drain, kit env incl. `WEEBO_OPERATOR_POD_LABELS`, `params.env`) |
| `weebo-operator.serviceAccount`            | `serviceAccount.create`                                       |
| `weebo-operator.clusterRole` / `…Binding`  | `params.kinds` (+ status, finalizers), Secrets, Events; with `watch.namespace`, the cluster-scoped kinds only |
| `weebo-operator.watchRole`                 | `watch.namespace`: `params.namespacedKinds`, Secrets, Events, `params.watchRules` in that namespace (`-watch`) |
| `weebo-operator.leaderElectionRole`        | Lease Role + RoleBinding in the release namespace             |
| `weebo-operator.namespacedExampleRole`     | `rbac.namespacedExample` on `params.namespacedKinds`          |
| `weebo-operator.service`                   | the webhook Service                                           |
| `weebo-operator.issuer` / `.certificate`   | cert-manager, when `webhook.certManager.enabled`              |
| `weebo-operator.validatingWebhookConfiguration` | `validate.<group>` on `params.admitted`; `scope.<group>` (`/scope`, UPDATE of `params.kinds`) with `watch.labelSelector`; nothing with `webhook.enabled: false` |
| `weebo-operator.podDisruptionBudget`       | when enabled and `replicaCount > 1`                           |
| `weebo-operator.metricsService`            | `metrics.prometheus.enabled`                                  |
| `weebo-operator.serviceMonitor`            | `serviceMonitor.enabled`                                      |
| `weebo-operator.prometheusRule`            | the kit's alerts (`<Chart>OperatorDown`, `NoLeader`, `MultipleLeaders`, `ReconcileErrors`, `ReconcileSlow`, `RepeatedDrift` unless `prometheusRule.repeatedDrift.enabled` is false: it reads `<prefix>_remote_writes_total`, exported only by an operator that calls `record_write`) + `params.alerts`; needs `metrics.prometheus.enabled`, scoped to the `<fullname>-metrics` scrape job (a ServiceMonitor on the metrics Service) |
| `weebo-operator.networkPolicy`             | `networkPolicy.enabled`, egress with `params.egress`          |
| `weebo-operator.argocdHealthConfigMap`     | the Ready-condition health check (Lua)                        |

Each takes `(dict "ctx" . "params" $params)`. Helpers take the root
context: `weebo-operator.fullname`, `.labels`, `.selectorLabels`,
`.serviceAccountName`, `.image`, `.leaseName`, `.webhookTlsSecretName`,
`.metricPrefix`, `.alertPrefix`, `.notes`, `.crdMetadata`,
`.argocdHealthLua`.

`task chart:check` (kit) lints this chart through the template's chart
and runs the shared smoke renders on it.
