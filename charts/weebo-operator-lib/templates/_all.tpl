{{/*
Every resource of an operator chart, from one parameter set — an operator
chart's whole templates/operator.yaml:

  {{ include "weebo-operator.all" (dict "ctx" . "params" (include "weebo-example.params" . | fromYaml)) }}

Resources a release does not enable render nothing; the others are
separated by `---`.
*/}}
{{- define "weebo-operator.all" -}}
{{- $docs := list -}}
{{- range $t := list "serviceAccount" "clusterRole" "clusterRoleBinding" "leaderElectionRole" "namespacedExampleRole" "service" "issuer" "certificate" "validatingWebhookConfiguration" "deployment" "podDisruptionBudget" "metricsService" "serviceMonitor" "prometheusRule" "networkPolicy" "argocdHealthConfigMap" -}}
{{- $doc := include (printf "weebo-operator.%s" $t) $ | trim -}}
{{- if $doc -}}
{{- $docs = append $docs $doc -}}
{{- end -}}
{{- end -}}
{{ join "\n---\n" $docs }}
{{- end -}}

{{/*
Header of a NOTES.txt: what is installed, its prerequisites, and the
warnings of the release's values. The operator's NOTES.txt follows with
its own first steps.
*/}}
{{- define "weebo-operator.notes" -}}
{{- $v := .Values -}}
{{ .Chart.Name }} {{ .Chart.AppVersion }} is installed ({{ $v.replicaCount }} replicas, Lease "{{ include "weebo-operator.leaseName" . }}").
{{- if $v.webhook.certManager.enabled }}

Prerequisite: cert-manager (it issues the admission webhook certificate).
{{- if $v.certManager.createIssuer }}
This release provisions its own self-signed Issuer
("{{ include "weebo-operator.webhookIssuerName" . }}").
{{- else }}
The Issuer/ClusterIssuer "{{ $v.certManager.issuerRef.name }}" ({{ $v.certManager.issuerRef.kind }}) must exist.
{{- end }}
{{- else }}

Webhook TLS: the Secret "{{ $v.webhook.tls.secretName }}" must exist in
{{ .Release.Namespace }} and stay signed by the CA in webhook.caBundle.
{{- end }}
{{- if not $v.crds.install }}

CRDs are NOT managed by this release (crds.install=false): apply the
crds.yaml of the matching GitHub release before using new fields.
{{- end }}
{{- if lt (int $v.replicaCount) 2 }}

WARNING: a single replica means no admission webhook (and so no namespaced
CR change, with failurePolicy {{ $v.webhook.failurePolicy }}) while it restarts.
{{- end }}
{{- end -}}

{{/*
Metadata of a CRD shipped as a template (crds.install), written by the
kit's `helm:crds` task: the chart's labels, and the keep policy — `helm
uninstall` never deletes a CRD, and with it every CR of the cluster.
*/}}
{{- define "weebo-operator.crdMetadata" -}}
labels:
  {{- include "weebo-operator.labels" . | nindent 2 }}
annotations:
  # Never deleted by helm uninstall: that would delete every CR.
  helm.sh/resource-policy: keep
{{- end -}}
