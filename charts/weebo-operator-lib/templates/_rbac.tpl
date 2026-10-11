{{/*
RBAC. Each takes (dict "ctx" $ "params" $params), see README.md.
*/}}

{{- define "weebo-operator.serviceAccount" -}}
{{- $ctx := .ctx -}}
{{- if $ctx.Values.serviceAccount.create -}}
apiVersion: v1
kind: ServiceAccount
metadata:
  name: {{ include "weebo-operator.serviceAccountName" $ctx }}
  labels:
    {{- include "weebo-operator.labels" $ctx | nindent 4 }}
  {{- with $ctx.Values.serviceAccount.annotations }}
  annotations:
    {{- toYaml . | nindent 4 }}
  {{- end }}
{{- end -}}
{{- end -}}

{{/*
The operator's own ClusterRole: its kinds (params.kinds, plurals of
params.group) watched cluster-wide with their status and finalizers,
Secrets read and written by name, Events. params.clusterRules appends
rules of its own.
*/}}
{{- define "weebo-operator.clusterRole" -}}
{{- $ctx := .ctx -}}
{{- $p := .params -}}
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: {{ include "weebo-operator.fullname" $ctx }}
  labels:
    {{- include "weebo-operator.labels" $ctx | nindent 4 }}
rules:
  # Every controller watches its kind cluster-wide (namespaced kinds
  # included) and the kinds it references.
  - apiGroups: [{{ $p.group | quote }}]
    resources:
      {{- range $p.kinds }}
      - {{ . }}
      {{- end }}
    verbs: ["get", "list", "watch", "update", "patch"]
  - apiGroups: [{{ $p.group | quote }}]
    resources:
      {{- range $p.kinds }}
      - {{ . }}/status
      {{- end }}
    verbs: ["get", "patch", "update"]
  - apiGroups: [{{ $p.group | quote }}]
    resources:
      {{- range $p.kinds }}
      - {{ . }}/finalizers
      {{- end }}
    verbs: ["update"]
  # Emitted credentials and the instances' own Secrets: read by name,
  # created, replaced whole when the operator owns them and deleted with
  # their CR. Never listed or watched.
  - apiGroups: [""]
    resources: ["secrets"]
    verbs: ["get", "create", "update", "patch", "delete"]
  # Kubernetes Events on every Ready transition and every failure.
  - apiGroups: ["events.k8s.io"]
    resources: ["events"]
    verbs: ["create", "patch"]
  {{- with $p.clusterRules }}
  {{- toYaml . | nindent 2 }}
  {{- end }}
{{- end -}}

{{- define "weebo-operator.clusterRoleBinding" -}}
{{- $ctx := .ctx -}}
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: {{ include "weebo-operator.fullname" $ctx }}
  labels:
    {{- include "weebo-operator.labels" $ctx | nindent 4 }}
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: {{ include "weebo-operator.fullname" $ctx }}
subjects:
  - kind: ServiceAccount
    name: {{ include "weebo-operator.serviceAccountName" $ctx }}
    namespace: {{ $ctx.Release.Namespace }}
{{- end -}}

{{/*
Leader election: a single Lease in the release namespace (POD_NAMESPACE),
hence a namespaced Role rather than cluster-wide Lease write access.
*/}}
{{- define "weebo-operator.leaderElectionRole" -}}
{{- $ctx := .ctx -}}
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: {{ include "weebo-operator.fullname" $ctx }}-leader-election
  namespace: {{ $ctx.Release.Namespace }}
  labels:
    {{- include "weebo-operator.labels" $ctx | nindent 4 }}
rules:
  - apiGroups: ["coordination.k8s.io"]
    resources: ["leases"]
    verbs: ["get", "create", "update", "patch"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: {{ include "weebo-operator.fullname" $ctx }}-leader-election
  namespace: {{ $ctx.Release.Namespace }}
  labels:
    {{- include "weebo-operator.labels" $ctx | nindent 4 }}
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: Role
  name: {{ include "weebo-operator.fullname" $ctx }}-leader-election
subjects:
  - kind: ServiceAccount
    name: {{ include "weebo-operator.serviceAccountName" $ctx }}
    namespace: {{ $ctx.Release.Namespace }}
{{- end -}}

{{/*
Optional namespaced Role(+Binding) EXAMPLE (rbac.namespacedExample): a
team manages the namespaced kinds (params.namespacedKinds) in its own
namespace; cluster-scoped kinds stay with cluster admins.
*/}}
{{- define "weebo-operator.namespacedExampleRole" -}}
{{- $ctx := .ctx -}}
{{- $p := .params -}}
{{- $ex := $ctx.Values.rbac.namespacedExample -}}
{{- if and $ex.enabled $p.namespacedKinds -}}
{{- $ns := $ex.namespace | default $ctx.Release.Namespace -}}
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: {{ include "weebo-operator.fullname" $ctx }}-author
  namespace: {{ $ns }}
  labels:
    {{- include "weebo-operator.labels" $ctx | nindent 4 }}
rules:
  - apiGroups: [{{ $p.group | quote }}]
    resources:
      {{- range $p.namespacedKinds }}
      - {{ . }}
      {{- end }}
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
  - apiGroups: [{{ $p.group | quote }}]
    resources:
      {{- range $p.namespacedKinds }}
      - {{ . }}/status
      {{- end }}
    verbs: ["get"]
{{- if $ex.subjects }}
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: {{ include "weebo-operator.fullname" $ctx }}-author
  namespace: {{ $ns }}
  labels:
    {{- include "weebo-operator.labels" $ctx | nindent 4 }}
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: Role
  name: {{ include "weebo-operator.fullname" $ctx }}-author
subjects:
  {{- toYaml $ex.subjects | nindent 2 }}
{{- end }}
{{- end -}}
{{- end -}}
