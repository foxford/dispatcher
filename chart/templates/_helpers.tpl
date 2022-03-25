{{/*
Expand the name of the chart.
*/}}
{{- define "dispatcher.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Service name.
*/}}
{{- define "dispatcher.serviceName" -}}
{{- list (default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-") "service" | join "-" }}
{{- end }}


{{/*
Short namespace.
*/}}
{{- define "dispatcher.shortNamespace" -}}
{{- $shortns := regexSplit "-" .Release.Namespace -1 | first }}
{{- if eq $shortns "production" }}
{{- else }}
{{- $shortns }}
{{- end }}
{{- end }}

{{/*
Namespace in ingress path.
converts as follows:
- testing01 -> t01
- staging01-classroom-ng -> s01/classroom-ng
- producion-webinar-ng -> webinar-ng
*/}}
{{- define "dispatcher.ingressPathNamespace" -}}
{{- $ns_head := regexSplit "-" .Release.Namespace -1 | first }}
{{- $ns_tail := regexSplit "-" .Release.Namespace -1 | rest | join "-" }}
{{- if eq $ns_head "production" }}
{{- regexReplaceAll "(.*)-ng(.*)" $ns_tail "${1}-foxford${2}" }}
{{- else }}
{{- $v := list (regexReplaceAll "(.)[^\\d]*(.+)" $ns_head "${1}${2}") $ns_tail | compact | join "/" }}
{{- regexReplaceAll "(.*)-ng(.*)" $v "${1}-foxford${2}" }}
{{- end }}
{{- end }}

{{/*
Ingress path.
*/}}
{{- define "dispatcher.ingressPath" -}}
{{- $shortns := regexSplit "-" .Release.Namespace -1 | first }}
{{- list "" (include "dispatcher.ingressPathNamespace" .) (include "dispatcher.name" .) | join "/" }}
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
app.kubernetes.io/name: {{ include "dispatcher.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
k8s-app: {{ include "dispatcher.name" . }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "dispatcher.selectorLabels" -}}
app.kubernetes.io/name: {{ include "dispatcher.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app: {{ include "dispatcher.name" . }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "dispatcher.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "dispatcher.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}
