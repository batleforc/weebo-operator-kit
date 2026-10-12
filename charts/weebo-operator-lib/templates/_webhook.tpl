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
the namespace allow-list admits), at params.versions. With a
watch.labelSelector, `scope.<params.group>` on UPDATE of every kind
(params.kinds): the kit's ScopeGuard (`/scope`) refuses taking a managed
object out of the selector. Their namespaceSelector/objectSelector
default to watch.namespace/watch.labelSelector. Nothing rendered when
there is neither, with webhook.enabled false (the reconcilers alone
enforce the rules) or watch.clusterKinds false (a cluster-scoped object).
*/}}
{{- define "weebo-operator.validatingWebhookConfiguration" -}}
{{- $ctx := .ctx -}}
{{- $p := .params -}}
{{- $wh := $ctx.Values.webhook -}}
{{- $watchNs := include "weebo-operator.watchNamespace" $ctx -}}
{{- $selector := include "weebo-operator.watchLabelSelector" $ctx -}}
{{- if and (include "weebo-operator.webhookEnabled" $ctx) (include "weebo-operator.watchClusterKinds" $ctx) (or $p.admitted $selector) -}}
{{- $namespaceSelector := $wh.namespaceSelector -}}
{{- if and (not $namespaceSelector) $watchNs -}}
{{- $namespaceSelector = dict "matchLabels" (dict "kubernetes.io/metadata.name" $watchNs) -}}
{{- end -}}
{{- $objectSelector := $wh.objectSelector -}}
{{- if and (not $objectSelector) $selector -}}
{{- $objectSelector = $selector | fromJson -}}
{{- end -}}
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
  {{- if $p.admitted }}
  - name: validate.{{ $p.group }}
    {{- include "weebo-operator.webhookCommon" (dict "ctx" $ctx "path" "/validate" "namespaceSelector" $namespaceSelector "objectSelector" $objectSelector) | nindent 4 }}
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
  {{- end }}
  {{- if $selector }}
  # An object is sent when its old or its new labels match the selector:
  # the one leaving it is.
  - name: scope.{{ $p.group }}
    {{- include "weebo-operator.webhookCommon" (dict "ctx" $ctx "path" "/scope" "namespaceSelector" $namespaceSelector "objectSelector" $objectSelector) | nindent 4 }}
    rules:
      - apiGroups: [{{ $p.group | quote }}]
        apiVersions: {{ $p.versions | default (list "v1alpha1") | toJson }}
        operations: ["UPDATE"]
        resources:
          {{- range $p.kinds }}
          - {{ . }}
          {{- end }}
  {{- end }}
{{- end -}}
{{- end -}}

{{/* What both webhooks share, `path` aside. */}}
{{- define "weebo-operator.webhookCommon" -}}
{{- $ctx := .ctx -}}
{{- $wh := $ctx.Values.webhook -}}
admissionReviewVersions: ["v1"]
sideEffects: None
failurePolicy: {{ $wh.failurePolicy }}
timeoutSeconds: {{ $wh.timeoutSeconds }}
matchPolicy: Equivalent
{{- with .namespaceSelector }}
namespaceSelector:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- with .objectSelector }}
objectSelector:
  {{- toYaml . | nindent 2 }}
{{- end }}
clientConfig:
  {{- if not $wh.certManager.enabled }}
  caBundle: {{ required "webhook.caBundle (base64 PEM of the CA that signed webhook.tls.secretName) is required when webhook.certManager.enabled is false" $wh.caBundle }}
  {{- end }}
  service:
    name: {{ include "weebo-operator.fullname" $ctx }}
    namespace: {{ $ctx.Release.Namespace }}
    path: {{ .path }}
    port: {{ $wh.servicePort }}
{{- end -}}
