{{- define "app_probes" }}
livenessProbe:
  tcpSocket:
    port: http
  initialDelaySeconds: 5
  periodSeconds: 10
readinessProbe:
  tcpSocket:
    port: http
  initialDelaySeconds: 15
  periodSeconds: 20
{{- end }}
