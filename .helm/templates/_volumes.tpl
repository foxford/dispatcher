{{- define "volumeMounts" }}
- name: config
  mountPath: /app/App.toml
  subPath: App.toml
- name: svc
  mountPath: /app/data/keys/svc.private_key.p8.der
  subPath: private_key
- name: svc
  mountPath: /app/data/keys/svc.public_key.p8.der
  subPath: public_key
{{- range $tenant := .Values.app.tenants }}
- name: {{ $tenant.name | lower }}
  mountPath: {{ printf "/app/%s" (pluck $.Values.werf.env $tenant.authn.key | first | default $tenant.authn.key._default) }}
  subPath: public_key
{{- end }}
{{- end }}

{{- define "volumes" }}
- name: config
  configMap:
    name: dispatcher-config
- name: svc
  secret:
    secretName: svc-pem-credentials
{{- range $tenant := .Values.app.tenants }}
- name: {{ $tenant.name | lower }}
  secret:
    secretName: secrets-{{ $tenant.name | lower }}
{{- end }}
{{- end }}
