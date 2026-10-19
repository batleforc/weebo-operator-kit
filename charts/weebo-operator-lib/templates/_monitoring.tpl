{{/*
Metrics: the plain-HTTP Prometheus Service, a ServiceMonitor and a
PrometheusRule. Each takes (dict "ctx" $ "params" $params).
*/}}

{{/*
Kept off the webhook Service so the API server's only path to the pods
stays the TLS webhook port.
*/}}
{{- define "weebo-operator.metricsService" -}}
{{- $ctx := .ctx -}}
{{- if include "weebo-operator.metricsEnabled" $ctx -}}
apiVersion: v1
kind: Service
metadata:
  name: {{ include "weebo-operator.fullname" $ctx }}-metrics
  labels:
    {{- include "weebo-operator.labels" $ctx | nindent 4 }}
    app.kubernetes.io/component: metrics
spec:
  selector:
    {{- include "weebo-operator.selectorLabels" $ctx | nindent 4 }}
  ports:
    - name: metrics
      port: {{ $ctx.Values.metrics.prometheus.port }}
      targetPort: metrics
      protocol: TCP
{{- end -}}
{{- end -}}

{{- define "weebo-operator.serviceMonitor" -}}
{{- $ctx := .ctx -}}
{{- $sm := $ctx.Values.serviceMonitor -}}
{{- if $sm.enabled -}}
{{- if not (include "weebo-operator.metricsEnabled" $ctx) }}
{{- fail "serviceMonitor.enabled needs metrics.prometheus.enabled" }}
{{- end -}}
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: {{ include "weebo-operator.fullname" $ctx }}
  labels:
    {{- include "weebo-operator.labels" $ctx | nindent 4 }}
    {{- with $sm.labels }}
    {{- toYaml . | nindent 4 }}
    {{- end }}
spec:
  namespaceSelector:
    matchNames:
      - {{ $ctx.Release.Namespace }}
  selector:
    matchLabels:
      {{- include "weebo-operator.selectorLabels" $ctx | nindent 6 }}
      app.kubernetes.io/component: metrics
  endpoints:
    - port: metrics
      path: /metrics
      scheme: http
      interval: {{ $sm.interval }}
      scrapeTimeout: {{ $sm.scrapeTimeout }}
      {{- with $sm.relabelings }}
      relabelings:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      {{- with $sm.metricRelabelings }}
      metricRelabelings:
        {{- toYaml . | nindent 8 }}
      {{- end }}
{{- end -}}
{{- end -}}

{{/*
The kit's alerts on the kit's metrics (`<prefix>_reconcile_total`,
`<prefix>_reconcile_duration_seconds`, `<prefix>_remote_writes_total`,
`<prefix>_leader`), prefix and
alert names from the chart name; params.alerts appends the operator's own
(a list of rules), before prometheusRule.additionalRules.
RepeatedDrift reads `<prefix>_remote_writes_total`, which only an operator
calling `ReconcileMetrics::record_write` exports: one that writes to no
remote system sets prometheusRule.repeatedDrift.enabled=false (default
true, also when the key is missing).

Metric names are matched with regexes: the operator exports through
OpenTelemetry, and the OTel -> Prometheus translation may append `_total`
to counters, a unit suffix (`_seconds`) to histograms and `_ratio` to
unit-less gauges, depending on the exporter version. Every series is
scoped to this release's scrape job, `<fullname>-metrics`: the job a
ServiceMonitor on the metrics Service gets (serviceMonitor.enabled, or one
of the cluster's own on that Service). Without the metrics, the absent()
alerts would fire forever: refused.
*/}}
{{- define "weebo-operator.prometheusRule" -}}
{{- $ctx := .ctx -}}
{{- $p := .params -}}
{{- $pr := $ctx.Values.prometheusRule -}}
{{- if $pr.enabled -}}
{{- if not (include "weebo-operator.metricsEnabled" $ctx) }}
{{- fail "prometheusRule.enabled needs metrics.prometheus.enabled" }}
{{- end -}}
{{- $m := include "weebo-operator.metricPrefix" $ctx -}}
{{- $a := include "weebo-operator.alertPrefix" $ctx -}}
{{- $sel := printf "namespace=%q, job=%q" $ctx.Release.Namespace (printf "%s-metrics" (include "weebo-operator.fullname" $ctx)) -}}
{{- $reconcile := printf "{__name__=~\"%s_reconcile(_total)*\", %s" $m $sel -}}
{{- $leader := printf "{__name__=~\"%s_leader(_ratio)?\", %s}" $m $sel -}}
{{- $duration := printf "{__name__=~\"%s_reconcile_duration(_seconds)*_bucket\", %s}" $m $sel -}}
{{- $writes := printf "{__name__=~\"%s_remote_writes(_total)*\", %s, cause=\"drift\"}" $m $sel -}}
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: {{ include "weebo-operator.fullname" $ctx }}
  labels:
    {{- include "weebo-operator.labels" $ctx | nindent 4 }}
    {{- with $pr.labels }}
    {{- toYaml . | nindent 4 }}
    {{- end }}
spec:
  groups:
    - name: {{ $ctx.Chart.Name }}
      rules:
        {{- if $pr.defaultRules }}
        - alert: {{ $a }}OperatorDown
          # No replica answers the scrape: with failurePolicy Fail, no
          # namespaced CR can be created or updated either.
          expr: absent(up{ {{- $sel -}} } == 1)
          for: 5m
          labels:
            severity: critical
          annotations:
            summary: No {{ $ctx.Chart.Name }} operator replica is up in {{ $ctx.Release.Namespace }}.
            description: The admission webhook is likely down too; namespaced CRs are refused while it is.
        - alert: {{ $a }}NoLeader
          expr: (sum({{ $leader }}) or vector(0)) < 1
          for: 5m
          labels:
            severity: critical
          annotations:
            summary: No {{ $ctx.Chart.Name }} replica holds the leader Lease in {{ $ctx.Release.Namespace }}.
            description: Nothing reconciles. Check the Lease {{ include "weebo-operator.leaseName" $ctx }} and the operator logs.
        - alert: {{ $a }}MultipleLeaders
          expr: sum({{ $leader }}) > 1
          for: 5m
          labels:
            severity: warning
          annotations:
            summary: Several {{ $ctx.Chart.Name }} replicas report being leader in {{ $ctx.Release.Namespace }}.
            description: Usually two releases sharing one Lease name, or a Lease rename during an upgrade.
        - alert: {{ $a }}ReconcileErrors
          expr: |
            sum by (kind) (rate({{ $reconcile }}, result=~"error|failure|failed"}[15m]))
              /
            sum by (kind) (rate({{ $reconcile }}}[15m]))
              > {{ $pr.reconcileErrorRatio }}
          for: 15m
          labels:
            severity: warning
          annotations:
            summary: {{`'{{ $labels.kind }} reconciles failing ({{ $value | humanizePercentage }})'`}}
            description: {{`'kubectl get {{ $labels.kind }} -A and read the Ready condition reason.'`}}
        - alert: {{ $a }}ReconcileSlow
          expr: |
            histogram_quantile(0.99, sum by (le, kind) (rate({{ $duration }}[15m]))) > 60
          for: 30m
          labels:
            severity: info
          annotations:
            summary: {{`'p99 reconcile of {{ $labels.kind }} above 60s'`}}
            description: The remote system answers slowly, or reconciles queue up.
        {{- if dig "repeatedDrift" "enabled" true $pr }}
        - alert: {{ $a }}RepeatedDrift
          # A real drift is corrected in one pass; a field the comparison
          # gets wrong (a value the remote system stores normalized) is
          # rewritten on every reconcile, ~6 times in 30 minutes.
          expr: sum by (kind) (increase({{ $writes }}[30m])) > 3
          labels:
            severity: warning
          annotations:
            summary: {{`'{{ $labels.kind }} objects rewritten on every reconcile'`}}
            description: {{`'Look for DriftCorrected Events on {{ $labels.kind }} naming the same field each time: a comparison to fix, not a drift.'`}}
        {{- end }}
        {{- with $p.alerts }}
        {{- toYaml . | nindent 8 }}
        {{- end }}
        {{- end }}
        {{- with $pr.additionalRules }}
        {{- toYaml . | nindent 8 }}
        {{- end }}
{{- end -}}
{{- end -}}
