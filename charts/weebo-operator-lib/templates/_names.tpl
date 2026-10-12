{{/*
Names and labels. Every template takes the operator chart's root context:
names come from the operator's own Chart.yaml, never from this chart.
*/}}

{{/* Chart name (nameOverride wins). */}}
{{- define "weebo-operator.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/* Chart name and version, for the helm.sh/chart label. */}}
{{- define "weebo-operator.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/* Full release name (fullnameOverride wins). */}}
{{- define "weebo-operator.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := include "weebo-operator.name" . -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{/* Common labels. */}}
{{- define "weebo-operator.labels" -}}
helm.sh/chart: {{ include "weebo-operator.chart" . }}
{{ include "weebo-operator.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{/* Selector labels. */}}
{{- define "weebo-operator.selectorLabels" -}}
app.kubernetes.io/name: {{ include "weebo-operator.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/* ServiceAccount name. */}}
{{- define "weebo-operator.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "weebo-operator.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- required "serviceAccount.name is required when serviceAccount.create is false" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{/*
Operator image: repository@digest when a digest is set, else
repository:tag (tag defaulting to `v` + appVersion, the release tag).
*/}}
{{- define "weebo-operator.image" -}}
{{- if .Values.image.digest -}}
{{- printf "%s@%s" .Values.image.repository .Values.image.digest -}}
{{- else -}}
{{- printf "%s:%s" .Values.image.repository (.Values.image.tag | default (printf "v%s" .Chart.AppVersion)) -}}
{{- end -}}
{{- end -}}

{{/*
Leader-election Lease name (env LEASE_NAME): `<chart>-operator-leader`
unless leaderElection.leaseName is set — the name the operator binary
uses by default.
*/}}
{{- define "weebo-operator.leaseName" -}}
{{- .Values.leaderElection.leaseName | default (printf "%s-operator-leader" .Chart.Name) -}}
{{- end -}}

{{/*
Webhook TLS Secret name: produced by the cert-manager Certificate, or
user-provided when cert-manager is disabled.
*/}}
{{- define "weebo-operator.webhookTlsSecretName" -}}
{{- if .Values.webhook.certManager.enabled -}}
{{- printf "%s-webhook-tls" (include "weebo-operator.fullname" .) -}}
{{- else -}}
{{- required "webhook.tls.secretName is required when webhook.certManager.enabled is false (a kubernetes.io/tls Secret valid for the webhook Service)" .Values.webhook.tls.secretName -}}
{{- end -}}
{{- end -}}

{{/* The self-signed Issuer provisioned when certManager.createIssuer. */}}
{{- define "weebo-operator.webhookIssuerName" -}}
{{- printf "%s-webhook" (include "weebo-operator.fullname" .) -}}
{{- end -}}

{{/* "true" when the Prometheus endpoint is served, else empty. */}}
{{- define "weebo-operator.metricsEnabled" -}}
{{- if .Values.metrics.prometheus.enabled -}}true{{- end -}}
{{- end -}}

{{/* Metric name prefix: the chart name, `-` → `_` (weebo_example). */}}
{{- define "weebo-operator.metricPrefix" -}}
{{- .Chart.Name | replace "-" "_" -}}
{{- end -}}

{{/* Alert name prefix: the chart name in CamelCase (WeeboExample). */}}
{{- define "weebo-operator.alertPrefix" -}}
{{- .Chart.Name | replace "-" " " | title | nospace -}}
{{- end -}}

{{/*
Watch scope (watch.*). Both render "" when unset: a chart whose values
have no `watch` block watches the whole cluster, as before.
*/}}
{{/* The namespace watched, "" for the whole cluster. */}}
{{- define "weebo-operator.watchNamespace" -}}
{{- (.Values.watch | default dict).namespace | default "" -}}
{{- end -}}

{{/* The label selector, "" when every object is watched. */}}
{{- define "weebo-operator.watchLabelSelector" -}}
{{- with (.Values.watch | default dict).labelSelector -}}
{{- if or .matchLabels .matchExpressions -}}{{ toJson . }}{{- end -}}
{{- end -}}
{{- end -}}

{{/* "true" unless watch.clusterKinds is false (full namespaced). */}}
{{- define "weebo-operator.watchClusterKinds" -}}
{{- if ne (.Values.watch | default dict).clusterKinds false -}}true{{- end -}}
{{- end -}}

{{/*
Fails the render on a watch scope that can't work: no cluster-scoped kind
needs a namespace, and the CRDs (cluster-scoped) managed out of band.
*/}}
{{- define "weebo-operator.checkWatch" -}}
{{- if not (include "weebo-operator.watchClusterKinds" .) -}}
{{- if not (include "weebo-operator.watchNamespace" .) -}}
{{- fail "watch.clusterKinds=false needs watch.namespace" -}}
{{- end -}}
{{- if .Values.crds.install -}}
{{- fail "watch.clusterKinds=false needs crds.install=false: CRDs are cluster-scoped, a cluster admin installs them" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{/* "true" unless webhook.enabled is false. */}}
{{- define "weebo-operator.webhookEnabled" -}}
{{- if ne .Values.webhook.enabled false -}}true{{- end -}}
{{- end -}}
