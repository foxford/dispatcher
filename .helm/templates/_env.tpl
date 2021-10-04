{{- define "init_app_envs" }}
- name: DATABASE_URL
  value: {{ pluck .Values.werf.env .Values.app.database_url | first | default .Values.app.database_url._default | quote }}
{{- end }}
{{- define "app_envs" }}
- name: RUST_LOG
  value: {{ pluck .Values.werf.env .Values.app.rust_log | first | default .Values.app.rust_log._default | quote }}
- name: DATABASE_POOL_SIZE
  value: {{ pluck .Values.werf.env .Values.app.database_pool_size | first | default .Values.app.database_pool_size._default | quote }}
- name: DATABASE_POOL_IDLE_SIZE
  value: {{ pluck .Values.werf.env .Values.app.database_pool_idle_size | first | default .Values.app.database_pool_idle_size._default | quote }}
- name: DATABASE_POOL_TIMEOUT
  value: {{ pluck .Values.werf.env .Values.app.database_pool_timeout | first | default .Values.app.database_pool_timeout._default | quote }}
- name: DATABASE_POOL_MAX_LIFETIME
  value: {{ pluck .Values.werf.env .Values.app.database_pool_max_lifetime | first | default .Values.app.database_pool_max_lifetime._default | quote }}
- name: DATABASE_URL
  value: {{ pluck .Values.werf.env .Values.app.database_url | first | default .Values.app.database_url._default | quote }}
- name: APP_AGENT_LABEL
  valueFrom:
    fieldRef:
      fieldPath: metadata.name
{{- end }}
