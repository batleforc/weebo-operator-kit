{{/*
The operator Deployment: every replica serves the admission webhook and
the health routes (HTTPS, port `webhook`), the elected leader reconciles.
Takes (dict "ctx" $ "params" $params); params.env adds the operator's own
environment variables before values.extraEnv.
*/}}
{{- define "weebo-operator.deployment" -}}
{{- $ctx := .ctx -}}
{{- $p := .params -}}
{{- $v := $ctx.Values -}}
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ include "weebo-operator.fullname" $ctx }}
  labels:
    {{- include "weebo-operator.labels" $ctx | nindent 4 }}
spec:
  replicas: {{ $v.replicaCount }}
  selector:
    matchLabels:
      {{- include "weebo-operator.selectorLabels" $ctx | nindent 6 }}
  template:
    metadata:
      labels:
        {{- include "weebo-operator.selectorLabels" $ctx | nindent 8 }}
        {{- with $v.podLabels }}
        {{- toYaml . | nindent 8 }}
        {{- end }}
      {{- with $v.podAnnotations }}
      annotations:
        {{- toYaml . | nindent 8 }}
      {{- end }}
    spec:
      serviceAccountName: {{ include "weebo-operator.serviceAccountName" $ctx }}
      {{- with $v.imagePullSecrets }}
      imagePullSecrets:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      {{- with $v.priorityClassName }}
      priorityClassName: {{ . }}
      {{- end }}
      # The leader drains its in-flight reconciles before exiting.
      terminationGracePeriodSeconds: {{ $v.shutdown.terminationGracePeriodSeconds }}
      {{- with $v.podSecurityContext }}
      securityContext:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      containers:
        - name: operator
          # Release images are tagged `v<version>` while appVersion carries
          # the bare version; image.digest wins.
          image: {{ include "weebo-operator.image" $ctx | quote }}
          imagePullPolicy: {{ $v.image.pullPolicy }}
          {{- with $v.securityContext }}
          securityContext:
            {{- toYaml . | nindent 12 }}
          {{- end }}
          ports:
            - name: webhook
              containerPort: {{ $v.webhook.containerPort }}
              protocol: TCP
            {{- if include "weebo-operator.metricsEnabled" $ctx }}
            - name: metrics
              containerPort: {{ $v.metrics.prometheus.port }}
              protocol: TCP
            {{- end }}
          env:
            # Scopes the leader-election Lease; HOSTNAME (the pod name) is
            # the holder identity.
            - name: POD_NAMESPACE
              valueFrom:
                fieldRef:
                  fieldPath: metadata.namespace
            - name: LEASE_NAME
              value: {{ include "weebo-operator.leaseName" $ctx | quote }}
            - name: WEBHOOK_PORT
              value: {{ $v.webhook.containerPort | quote }}
            - name: WEEBO_SHUTDOWN_DRAIN_SECONDS
              value: {{ $v.shutdown.drainSeconds | quote }}
            - name: WEEBO_RECONCILE_CONCURRENCY
              value: {{ $v.reconcileConcurrency | quote }}
            - name: LOG_FORMAT
              value: {{ $v.logFormat | quote }}
            {{- if include "weebo-operator.metricsEnabled" $ctx }}
            - name: WEEBO_METRICS_ADDR
              value: {{ printf "0.0.0.0:%v" $v.metrics.prometheus.port | quote }}
            {{- end }}
            {{- with $p.env }}
            {{- toYaml . | nindent 12 }}
            {{- end }}
            {{- with $v.extraEnv }}
            {{- toYaml . | nindent 12 }}
            {{- end }}
          volumeMounts:
            # Default WEBHOOK_TLS_CERT_FILE/KEY_FILE location.
            - name: webhook-tls
              mountPath: /etc/webhook/certs
              readOnly: true
            # A Vault CA is materialized to a temp file for the Vault
            # client: /tmp must be writable under readOnlyRootFilesystem.
            - name: tmp
              mountPath: /tmp
          {{- with $v.probes }}
          startupProbe:
            httpGet:
              path: /readyz
              port: webhook
              scheme: HTTPS
            periodSeconds: {{ .startup.periodSeconds }}
            timeoutSeconds: {{ .startup.timeoutSeconds }}
            failureThreshold: {{ .startup.failureThreshold }}
          readinessProbe:
            httpGet:
              path: /readyz
              port: webhook
              scheme: HTTPS
            periodSeconds: {{ .readiness.periodSeconds }}
            timeoutSeconds: {{ .readiness.timeoutSeconds }}
            failureThreshold: {{ .readiness.failureThreshold }}
          livenessProbe:
            httpGet:
              path: /livez
              port: webhook
              scheme: HTTPS
            periodSeconds: {{ .liveness.periodSeconds }}
            timeoutSeconds: {{ .liveness.timeoutSeconds }}
            failureThreshold: {{ .liveness.failureThreshold }}
          {{- end }}
          {{- if gt (int $v.shutdown.preStopSleepSeconds) 0 }}
          # Native sleep action (Kubernetes 1.30+): the images have no shell.
          lifecycle:
            preStop:
              sleep:
                seconds: {{ $v.shutdown.preStopSleepSeconds }}
          {{- end }}
          {{- with $v.resources }}
          resources:
            {{- toYaml . | nindent 12 }}
          {{- end }}
      volumes:
        - name: webhook-tls
          secret:
            secretName: {{ include "weebo-operator.webhookTlsSecretName" $ctx }}
        - name: tmp
          emptyDir: {}
      {{- with $v.nodeSelector }}
      nodeSelector:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      {{- with $v.tolerations }}
      tolerations:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      {{- with $v.affinity }}
      affinity:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      {{- if $v.topologySpreadConstraints }}
      topologySpreadConstraints:
        {{- range $v.topologySpreadConstraints }}
        {{- $c := deepCopy . }}
        {{- if not $c.labelSelector }}
        {{- $_ := set $c "labelSelector" (dict "matchLabels" (include "weebo-operator.selectorLabels" $ctx | fromYaml)) }}
        {{- end }}
        - {{ toYaml $c | nindent 10 | trim }}
        {{- end }}
      {{- else if $v.defaultTopologySpread }}
      topologySpreadConstraints:
        - maxSkew: 1
          topologyKey: kubernetes.io/hostname
          whenUnsatisfiable: ScheduleAnyway
          labelSelector:
            matchLabels:
              {{- include "weebo-operator.selectorLabels" $ctx | nindent 14 }}
      {{- end }}
{{- end -}}

{{/*
Only rendered when replicaCount > 1: with a single replica, minAvailable 1
would block every node drain.
*/}}
{{- define "weebo-operator.podDisruptionBudget" -}}
{{- $ctx := .ctx -}}
{{- if and $ctx.Values.podDisruptionBudget.enabled (gt (int $ctx.Values.replicaCount) 1) -}}
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: {{ include "weebo-operator.fullname" $ctx }}
  labels:
    {{- include "weebo-operator.labels" $ctx | nindent 4 }}
spec:
  minAvailable: {{ $ctx.Values.podDisruptionBudget.minAvailable }}
  selector:
    matchLabels:
      {{- include "weebo-operator.selectorLabels" $ctx | nindent 6 }}
{{- end -}}
{{- end -}}

{{/*
Optional NetworkPolicy. Ingress: the webhook port and the metrics port.
Egress is only restricted when networkPolicy.egress.enabled: DNS, the API
server, then params.egress (the remote system, as
`[{comment, rules}]`), networkPolicy.egress.vault and .extra.
*/}}
{{- define "weebo-operator.networkPolicy" -}}
{{- $ctx := .ctx -}}
{{- $p := .params -}}
{{- $np := $ctx.Values.networkPolicy -}}
{{- if $np.enabled -}}
{{- if and $np.egress.enabled (not $np.egress.apiServer.to) }}
{{- fail "networkPolicy.egress.apiServer.to is required when networkPolicy.egress.enabled: the operator cannot work without the API server" }}
{{- end -}}
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: {{ include "weebo-operator.fullname" $ctx }}
  labels:
    {{- include "weebo-operator.labels" $ctx | nindent 4 }}
spec:
  podSelector:
    matchLabels:
      {{- include "weebo-operator.selectorLabels" $ctx | nindent 6 }}
  policyTypes:
    - Ingress
    {{- if $np.egress.enabled }}
    - Egress
    {{- end }}
  ingress:
    # Admission requests from the API server, and the probes (kubelet
    # traffic is not subject to NetworkPolicy on most CNIs).
    - ports:
        - port: webhook
          protocol: TCP
      {{- with $np.webhookFrom }}
      from:
        {{- toYaml . | nindent 8 }}
      {{- end }}
    {{- if include "weebo-operator.metricsEnabled" $ctx }}
    - ports:
        - port: metrics
          protocol: TCP
      {{- with $np.metricsFrom }}
      from:
        {{- toYaml . | nindent 8 }}
      {{- end }}
    {{- end }}
  {{- if $np.egress.enabled }}
  egress:
    {{- if $np.egress.dns.enabled }}
    - to:
        {{- toYaml $np.egress.dns.to | nindent 8 }}
      ports:
        {{- toYaml $np.egress.dns.ports | nindent 8 }}
    {{- end }}
    # Watches, Secrets, Leases, Events.
    - to:
        {{- toYaml $np.egress.apiServer.to | nindent 8 }}
      ports:
        {{- toYaml $np.egress.apiServer.ports | nindent 8 }}
    {{- range $e := $p.egress }}
    {{- if $e.rules }}
    {{- with $e.comment }}
    # {{ . }}
    {{- end }}
    {{- toYaml $e.rules | nindent 4 }}
    {{- end }}
    {{- end }}
    {{- with $np.egress.vault }}
    # Vault secret stores.
    {{- toYaml . | nindent 4 }}
    {{- end }}
    {{- with $np.egress.extra }}
    {{- toYaml . | nindent 4 }}
    {{- end }}
  {{- end }}
{{- end -}}
{{- end -}}
