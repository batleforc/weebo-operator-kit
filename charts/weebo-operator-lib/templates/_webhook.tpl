{{/*
Admission webhook: its Service, its certificate (cert-manager, or a
Secret and CA bundle of the user's) and the ValidatingWebhookConfiguration.
Each takes (dict "ctx" $ "params" $params), see README.md.
*/}}

{{- define "weebo-operator.service" -}}
{{- $ctx := .ctx -}}
apiVersion: v1
kind: Service
metadata:
  name: {{ include "weebo-operator.fullname" $ctx }}
  labels:
    {{- include "weebo-operator.labels" $ctx | nindent 4 }}
    app.kubernetes.io/component: webhook
spec:
  selector:
    {{- include "weebo-operator.selectorLabels" $ctx | nindent 4 }}
  ports:
    - name: webhook
      port: {{ $ctx.Values.webhook.servicePort }}
      targetPort: webhook
      protocol: TCP
{{- end -}}

{{/*
Self-signed Issuer for the webhook certificate, so no pre-existing
Issuer/ClusterIssuer is required (cert-manager itself must be installed):
the webhook's caBundle is injected from the Certificate's CA.
*/}}
{{- define "weebo-operator.issuer" -}}
{{- $ctx := .ctx -}}
{{- if and $ctx.Values.webhook.certManager.enabled $ctx.Values.certManager.createIssuer -}}
apiVersion: cert-manager.io/v1
kind: Issuer
metadata:
  name: {{ include "weebo-operator.webhookIssuerName" $ctx }}
  labels:
    {{- include "weebo-operator.labels" $ctx | nindent 4 }}
spec:
  selfSigned: {}
{{- end -}}
{{- end -}}

{{- define "weebo-operator.certificate" -}}
{{- $ctx := .ctx -}}
{{- if $ctx.Values.webhook.certManager.enabled -}}
{{- $cm := $ctx.Values.certManager -}}
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: {{ include "weebo-operator.fullname" $ctx }}-webhook
  labels:
    {{- include "weebo-operator.labels" $ctx | nindent 4 }}
spec:
  secretName: {{ include "weebo-operator.webhookTlsSecretName" $ctx }}
  duration: {{ $cm.duration }}
  renewBefore: {{ $cm.renewBefore }}
  dnsNames:
    - {{ include "weebo-operator.fullname" $ctx }}.{{ $ctx.Release.Namespace }}.svc
    - {{ include "weebo-operator.fullname" $ctx }}.{{ $ctx.Release.Namespace }}.svc.cluster.local
  issuerRef:
    {{- if $cm.createIssuer }}
    name: {{ include "weebo-operator.webhookIssuerName" $ctx }}
    kind: Issuer
    {{- else }}
    name: {{ required "certManager.issuerRef.name is required when certManager.createIssuer is false (an existing Issuer/ClusterIssuer)" $cm.issuerRef.name }}
    kind: {{ $cm.issuerRef.kind }}
    {{- end }}
{{- end -}}
{{- end -}}

{{/*
`validate.<params.group>` on CREATE/UPDATE of params.admitted (the kinds
the namespace allow-list admits), at params.versions. Nothing rendered
when params.admitted is empty.
*/}}
{{- define "weebo-operator.validatingWebhookConfiguration" -}}
{{- $ctx := .ctx -}}
{{- $p := .params -}}
{{- $wh := $ctx.Values.webhook -}}
{{- if $p.admitted -}}
apiVersion: admissionregistration.k8s.io/v1
kind: ValidatingWebhookConfiguration
metadata:
  name: {{ include "weebo-operator.fullname" $ctx }}
  labels:
    {{- include "weebo-operator.labels" $ctx | nindent 4 }}
  {{- if or $wh.certManager.enabled $wh.annotations }}
  annotations:
    {{- if $wh.certManager.enabled }}
    # cert-manager's CA injector fills caBundle from the Certificate.
    cert-manager.io/inject-ca-from: {{ $ctx.Release.Namespace }}/{{ include "weebo-operator.fullname" $ctx }}-webhook
    {{- end }}
    {{- with $wh.annotations }}
    {{- toYaml . | nindent 4 }}
    {{- end }}
  {{- end }}
webhooks:
  - name: validate.{{ $p.group }}
    admissionReviewVersions: ["v1"]
    sideEffects: None
    failurePolicy: {{ $wh.failurePolicy }}
    timeoutSeconds: {{ $wh.timeoutSeconds }}
    matchPolicy: Equivalent
    {{- with $wh.namespaceSelector }}
    namespaceSelector:
      {{- toYaml . | nindent 6 }}
    {{- end }}
    {{- with $wh.objectSelector }}
    objectSelector:
      {{- toYaml . | nindent 6 }}
    {{- end }}
    clientConfig:
      {{- if not $wh.certManager.enabled }}
      caBundle: {{ required "webhook.caBundle (base64 PEM of the CA that signed webhook.tls.secretName) is required when webhook.certManager.enabled is false" $wh.caBundle }}
      {{- end }}
      service:
        name: {{ include "weebo-operator.fullname" $ctx }}
        namespace: {{ $ctx.Release.Namespace }}
        path: /validate
        port: {{ $wh.servicePort }}
    rules:
      # CREATE/UPDATE only, never DELETE: revoking a namespace's access must
      # never block the finalizer cleanup of what it already created. The
      # finalizer removal itself is an UPDATE, so the webhook lets through
      # any UPDATE of an object that is already terminating.
      - apiGroups: [{{ $p.group | quote }}]
        apiVersions: {{ $p.versions | default (list "v1alpha1") | toJson }}
        operations: ["CREATE", "UPDATE"]
        resources:
          {{- range $p.admitted }}
          - {{ . }}
          {{- end }}
{{- end -}}
{{- end -}}
