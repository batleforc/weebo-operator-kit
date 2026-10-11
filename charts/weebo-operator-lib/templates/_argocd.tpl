{{/*
Argo CD health check for every kind of the operator (they all carry the
kit's status.conditions[type=Ready]). The ConfigMap only *ships* the
script: Argo CD reads its resource customizations from its own argocd-cm,
which a chart has no business writing to (see the operator's NOTES.txt).
*/}}
{{- define "weebo-operator.argocdHealthConfigMap" -}}
{{- $ctx := .ctx -}}
apiVersion: v1
kind: ConfigMap
metadata:
  name: {{ include "weebo-operator.fullname" $ctx }}-argocd-health
  labels:
    {{- include "weebo-operator.labels" $ctx | nindent 4 }}
data:
  health.lua: |
    {{- include "weebo-operator.argocdHealthLua" $ctx | nindent 4 }}
{{- end -}}

{{/* The Lua itself, for a NOTES.txt to print. */}}
{{- define "weebo-operator.argocdHealthLua" -}}
hs = {}
if obj.status ~= nil and obj.status.conditions ~= nil then
  for i, condition in ipairs(obj.status.conditions) do
    if condition.type == "Ready" then
      if condition.status == "True" then
        hs.status = "Healthy"
      elseif condition.status == "False" then
        hs.status = "Degraded"
      else
        hs.status = "Progressing"
      end
      hs.message = condition.message
      return hs
    end
  end
end
hs.status = "Progressing"
hs.message = "Waiting for Ready condition"
return hs
{{- end -}}
