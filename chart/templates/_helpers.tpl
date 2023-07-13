{{/*
Expand the name of the chart.
*/}}
{{- define "dispatcher.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "dispatcher.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "dispatcher.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "dispatcher.labels" -}}
helm.sh/chart: {{ include "dispatcher.chart" . }}
app.kubernetes.io/version: {{ .Values.app.image.tag | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{ include "dispatcher.selectorLabels" . }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "dispatcher.selectorLabels" -}}
app.kubernetes.io/name: {{ include "dispatcher.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Tenant Service Audience
*/}}
{{- define "dispatcher.tenantServiceAudience" -}}
{{- $tenant := . -}}
{{- list "svc" $tenant | join "." -}}
{{- end -}}

{{/*
Tenant User Audience
*/}}
{{- define "dispatcher.tenantUserAudience" -}}
{{- $tenant := . -}}
{{- list "usr" $tenant | join "." -}}
{{- end -}}

{{/*
Tenant Object Audience
*/}}
{{- define "dispatcher.tenantObjectAudience" -}}
{{- $namespace := index . 0 -}}
{{- $tenant := index . 1 -}}
{{- $env := regexSplit "-" $namespace -1 | first -}}
{{- $devEnv := ""}}
{{- if ne $env "p" }}
{{- $devEnv = regexReplaceAll "(s)(\\d\\d)" $env "staging${2}" }}
{{- $devEnv = regexReplaceAll "(t)(\\d\\d)" $devEnv "testing${2}" }}
{{- end }}
{{- list $devEnv $tenant | compact | join "." }}
{{- end }}

{{/*
Namespace in ingress path.
converts as follows:
- testing01 -> t01
- staging01-classroom-ng -> s01/classroom-foxford
- production-webinar-ng -> webinar-foxford
*/}}
{{- define "dispatcher.ingressPathNamespace" -}}
{{- $ns_head := regexSplit "-" .Release.Namespace -1 | first }}
{{- $ns_tail := regexSplit "-" .Release.Namespace -1 | rest | join "-" | replace "ng" "foxford" }}
{{- if has $ns_head (list "production" "p") }}
{{- $ns_tail }}
{{- else }}
{{- list (regexReplaceAll "(.)[^\\d]*(.+)" $ns_head "${1}${2}") $ns_tail | compact | join "/" }}
{{- end }}
{{- end }}

{{/*
Ingress path.
*/}}
{{- define "dispatcher.ingressPath" -}}
{{- list "" (include "dispatcher.ingressPathNamespace" .) (include "dispatcher.fullname" .) | join "/" }}
{{- end }}

{{/*
Storage url.
*/}}
{{- define "dispatcher.defaultStorageUrl" -}}
    {{ printf "https://%s/%s/storage" .Values.ingress.host (include "dispatcher.ingressPathNamespace" .) | quote }}
{{- end }}

{{/*
Create volumeMount name from audience and secret name
*/}}
{{- define "dispatcher.volumeMountName" -}}
{{- $audience := index . 0 -}}
{{- $secret := index . 1 -}}
{{- printf "%s-%s-secret" $audience $secret | replace "." "-" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Generate path to file in S3 `apps-data` bucket
*/}}
{{- define "dispatcher.appsDataS3Path" -}}
{{- $namespace := index . 0 -}}
{{- $tenant := index . 1 -}}
{{- $file := index . 2 -}}
{{- $namespaceEnv := regexSplit "-" $namespace -1 | first -}}
{{- $env := "dev"}}
{{- if eq $namespaceEnv "p" }}
{{- $env = "prod"}}
{{- end }}
{{- list "s3://apps-data" $env "defaults" $tenant $file | compact | join "/" }}
{{- end }}
