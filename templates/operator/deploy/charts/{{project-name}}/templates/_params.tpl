{% raw %}{{/*
This operator's parameters for the kit's library chart (weebo-operator-lib,
see its README): everything templates/operator.yaml renders that is not a
value.
*/}}
{{- define "operator.params" -}}{% endraw %}
group: {{product | downcase}}.weebo.io
versions: [v1alpha1]
# Every kind the operator watches (ClusterRole).
kinds:
  - {{product | downcase}}instances
  - {{product | downcase}}localinstances
  - {{product | downcase}}objects
# The namespaced kinds: what a team may manage (rbac.namespacedExample),
# and the watch Role's (watch.namespace).
namespacedKinds:
  - {{product | downcase}}localinstances
  - {{product | downcase}}objects
# Kinds the admission webhook checks: none until the operator serves
# `/validate` (a namespace allow-list).
admitted: []
# The operator's own environment variables.
env: []
egress:
  - comment: {{product}} instances.
    rules: {% raw %}{{ toJson .Values.networkPolicy.egress.{% endraw %}{{product | downcase}}{% raw %} }}{% endraw %}
# Alerts on the operator's own metrics, appended to the kit's.
alerts: []
{% raw %}{{- end -}}{% endraw %}
