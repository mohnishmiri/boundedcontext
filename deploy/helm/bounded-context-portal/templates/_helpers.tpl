{{/* ---------------------------------------------------------------- names */}}

{{- define "bcp.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/* Resource prefix: fullnameOverride or the release name (bcp -> bcp-api, bcp-worker, ...). */}}
{{- define "bcp.fullname" -}}
{{- default .Release.Name .Values.fullnameOverride | trunc 40 | trimSuffix "-" -}}
{{- end -}}

{{- define "bcp.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/* ---------------------------------------------------------------- labels */}}

{{- define "bcp.labels" -}}
helm.sh/chart: {{ include "bcp.chart" . }}
app.kubernetes.io/name: {{ include "bcp.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: bounded-context-portal
{{- end -}}

{{- define "bcp.selectorLabels" -}}
app.kubernetes.io/name: {{ include "bcp.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/* usage: include "bcp.componentLabels" (dict "ctx" $ "component" "api") */}}
{{- define "bcp.componentLabels" -}}
{{ include "bcp.labels" .ctx }}
app.kubernetes.io/component: {{ .component }}
{{- end -}}

{{- define "bcp.componentSelectorLabels" -}}
{{ include "bcp.selectorLabels" .ctx }}
app.kubernetes.io/component: {{ .component }}
{{- end -}}

{{/* ---------------------------------------------------------------- images */}}

{{/* usage: include "bcp.image" (dict "ctx" $ "image" .Values.api.image) */}}
{{- define "bcp.image" -}}
{{- $registry := .ctx.Values.global.imageRegistry -}}
{{- $repo := .image.repository -}}
{{- if $registry }}{{ $repo = printf "%s/%s" (trimSuffix "/" $registry) $repo }}{{ end -}}
{{- if .image.digest -}}
{{- printf "%s@%s" $repo .image.digest -}}
{{- else -}}
{{- printf "%s:%s" $repo (default .ctx.Chart.AppVersion .image.tag) -}}
{{- end -}}
{{- end -}}

{{- define "bcp.imagePullSecrets" -}}
{{- with .Values.global.imagePullSecrets }}
imagePullSecrets:
{{- toYaml . | nindent 2 }}
{{- end }}
{{- end -}}

{{/* ---------------------------------------------------------------- security */}}

{{- define "bcp.podSecurityContext" -}}
runAsNonRoot: true
runAsUser: {{ .Values.podSecurity.runAsUser }}
runAsGroup: {{ .Values.podSecurity.runAsGroup }}
fsGroup: {{ .Values.podSecurity.fsGroup }}
fsGroupChangePolicy: OnRootMismatch
seccompProfile:
  type: RuntimeDefault
{{- end -}}

{{- define "bcp.containerSecurityContext" -}}
runAsNonRoot: true
readOnlyRootFilesystem: true
allowPrivilegeEscalation: false
privileged: false
capabilities:
  drop: ["ALL"]
seccompProfile:
  type: RuntimeDefault
{{- end -}}

{{/* ---------------------------------------------------------------- service accounts / identity */}}

{{- define "bcp.migrateClientId" -}}
{{- default .Values.serviceAccounts.api.clientId .Values.serviceAccounts.migrate.clientId -}}
{{- end -}}

{{/* usage: include "bcp.podIdentityLabels" $ */}}
{{- define "bcp.podIdentityLabels" -}}
{{- if .Values.workloadIdentity.podLabel }}
azure.workload.identity/use: "true"
{{- end }}
{{- end -}}

{{/* ---------------------------------------------------------------- secrets */}}

{{- define "bcp.syncedDbSecretName" -}}
{{- default (printf "%s-db-connection" (include "bcp.fullname" .)) .Values.secrets.syncedDbSecretName -}}
{{- end -}}

{{/* SecretProviderClass `objects` parameter. usage: include "bcp.spcObjects" (dict "names" (list "a" "b")) */}}
{{- define "bcp.spcObjects" -}}
array:
{{- range .names }}
  - |
    objectName: {{ . }}
    objectType: secret
{{- end }}
{{- end -}}

{{- define "bcp.kedaConnectionSecretName" -}}
{{- default (include "bcp.syncedDbSecretName" .) .Values.keda.connectionSecret.name -}}
{{- end -}}

{{/* usage: include "bcp.secretVolume" (dict "ctx" $ "component" "api") — component: api | worker | migrate */}}
{{- define "bcp.secretVolume" -}}
{{- $v := .ctx.Values -}}
- name: secrets
{{- if eq $v.secrets.mode "csi" }}
  csi:
    driver: secrets-store.csi.k8s.io
    readOnly: true
    volumeAttributes:
      secretProviderClass: {{ printf "%s-%s" (include "bcp.fullname" .ctx) .component }}
{{- else }}
  secret:
    secretName: {{ required "secrets.existingSecret is required when secrets.mode=existingSecret" $v.secrets.existingSecret }}
    defaultMode: 0440
{{- end }}
{{- end -}}

{{/* *_FILE env vars. usage: include "bcp.secretFileEnv" (dict "ctx" $ "all" true) — migrate only needs the DB URL */}}
{{- define "bcp.secretFileEnv" -}}
{{- $s := .ctx.Values.secrets -}}
- name: BCP_DATABASE_URL_FILE
  value: {{ printf "%s/%s" $s.mountPath $s.keyVault.objects.databaseUrl | quote }}
{{- if .all }}
- name: BCP_GITHUB_CLIENT_SECRET_FILE
  value: {{ printf "%s/%s" $s.mountPath $s.keyVault.objects.githubClientSecret | quote }}
- name: BCP_GITHUB_PRIVATE_KEY_FILE
  value: {{ printf "%s/%s" $s.mountPath $s.keyVault.objects.githubAppPrivateKey | quote }}
- name: BCP_TOKEN_ENCRYPTION_KEY_FILE
  value: {{ printf "%s/%s" $s.mountPath $s.keyVault.objects.tokenEncryptionKey | quote }}
{{- end }}
{{- end -}}

{{/* ---------------------------------------------------------------- shared config volumes */}}

{{/* Volumes for models/agent-bundle overrides (api + worker). */}}
{{- define "bcp.overrideVolumes" -}}
{{- if .Values.models.content }}
- name: models
  configMap:
    name: {{ include "bcp.fullname" . }}-models
{{- end }}
{{- if .Values.generators.content }}
- name: generators
  configMap:
    name: {{ include "bcp.fullname" . }}-generators
{{- end }}
{{- if .Values.agentBundle.configMap }}
- name: agent-bundle
  configMap:
    name: {{ .Values.agentBundle.configMap }}
    {{- with .Values.agentBundle.items }}
    items:
      {{- toYaml . | nindent 6 }}
    {{- end }}
{{- end }}
{{- end -}}

{{- define "bcp.overrideVolumeMounts" -}}
{{- if .Values.models.content }}
- name: models
  mountPath: {{ .Values.config.modelsFile }}
  subPath: models.yaml
  readOnly: true
{{- end }}
{{- if .Values.generators.content }}
- name: generators
  mountPath: {{ .Values.config.generatorsFile }}
  subPath: generators.yaml
  readOnly: true
{{- end }}
{{- if .Values.agentBundle.configMap }}
- name: agent-bundle
  mountPath: {{ .Values.config.agentBundleDir }}
  readOnly: true
{{- end }}
{{- end -}}

{{/* Checksum annotations shared by api and worker so config changes roll both. */}}
{{- define "bcp.sharedChecksums" -}}
checksum/models: {{ .Values.models.content | sha256sum | trunc 16 }}
checksum/generators: {{ .Values.generators.content | sha256sum | trunc 16 }}
checksum/agent-bundle: {{ printf "%s/%s/%s" .Values.agentBundle.configMap (toJson .Values.agentBundle.items) .Values.agentBundle.checksum | sha256sum | trunc 16 }}
checksum/secrets: {{ toJson .Values.secrets | sha256sum | trunc 16 }}
{{- end -}}

{{- define "bcp.metricsAnnotations" -}}
{{- if .ctx.Values.metrics.podAnnotations }}
prometheus.io/scrape: "true"
prometheus.io/port: {{ .port | quote }}
prometheus.io/path: /metrics
{{- end }}
{{- end -}}

{{/* ---------------------------------------------------------------- scheduling */}}

{{/* usage: include "bcp.topologySpread" (dict "ctx" $ "component" "api") */}}
{{- define "bcp.topologySpread" -}}
{{- if .ctx.Values.topologySpread.enabled }}
topologySpreadConstraints:
  - maxSkew: {{ .ctx.Values.topologySpread.zoneSkew }}
    topologyKey: topology.kubernetes.io/zone
    whenUnsatisfiable: ScheduleAnyway
    labelSelector:
      matchLabels:
        {{- include "bcp.componentSelectorLabels" . | nindent 8 }}
  - maxSkew: {{ .ctx.Values.topologySpread.hostSkew }}
    topologyKey: kubernetes.io/hostname
    whenUnsatisfiable: ScheduleAnyway
    labelSelector:
      matchLabels:
        {{- include "bcp.componentSelectorLabels" . | nindent 8 }}
{{- end }}
{{- end -}}

{{/* ---------------------------------------------------------------- worker pod spec */}}

{{/*
Worker pod spec shared by the Deployment (loop) and the KEDA ScaledJob (once).
usage: include "bcp.worker.podSpec" (dict "ctx" $ "once" false)
*/}}
{{- define "bcp.worker.podSpec" -}}
{{- $ctx := .ctx -}}
{{- $w := $ctx.Values.worker -}}
serviceAccountName: {{ $ctx.Values.serviceAccounts.worker.name }}
automountServiceAccountToken: false
{{- with (include "bcp.imagePullSecrets" $ctx | trim) }}{{ . | nindent 0 }}{{- end }}
securityContext:
  {{- include "bcp.podSecurityContext" $ctx | nindent 2 }}
terminationGracePeriodSeconds: {{ $w.terminationGracePeriodSeconds }}
{{- if .once }}
restartPolicy: Never
{{- else }}
{{- with (include "bcp.topologySpread" (dict "ctx" $ctx "component" "worker") | trim) }}{{ . | nindent 0 }}{{- end }}
{{- end }}
{{- with $w.nodeSelector }}
nodeSelector:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- with $w.tolerations }}
tolerations:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- with $w.affinity }}
affinity:
  {{- toYaml . | nindent 2 }}
{{- end }}
containers:
  - name: worker
    image: {{ include "bcp.image" (dict "ctx" $ctx "image" $w.image) }}
    imagePullPolicy: {{ $ctx.Values.image.pullPolicy }}
    args: [{{ if .once }}"once"{{ else }}"run"{{ end }}]
    envFrom:
      - configMapRef:
          name: {{ include "bcp.fullname" $ctx }}-worker
    env:
      - name: BCP_WORKER_MODE
        value: {{ if .once }}"once"{{ else }}"loop"{{ end }}
      - name: BCP_WORKER_ID
        valueFrom:
          fieldRef:
            fieldPath: metadata.name
      {{- include "bcp.secretFileEnv" (dict "ctx" $ctx "all" true) | nindent 6 }}
    {{- if not .once }}
    ports:
      - name: http
        containerPort: 8081
        protocol: TCP
      - name: metrics
        containerPort: 9091
        protocol: TCP
    livenessProbe:
      httpGet: { path: /healthz, port: http }
      initialDelaySeconds: 5
      periodSeconds: 15
      timeoutSeconds: 3
      failureThreshold: 4
    readinessProbe:
      httpGet: { path: /readyz, port: http }
      initialDelaySeconds: 5
      periodSeconds: 10
      timeoutSeconds: 3
      failureThreshold: 3
    {{- end }}
    resources:
      {{- toYaml $w.resources | nindent 6 }}
    securityContext:
      {{- include "bcp.containerSecurityContext" $ctx | nindent 6 }}
    volumeMounts:
      - name: secrets
        mountPath: {{ $ctx.Values.secrets.mountPath }}
        readOnly: true
      - name: work
        mountPath: {{ $w.workDir.path }}
      - name: tmp
        mountPath: /tmp
      - name: home
        mountPath: /home/worker
      {{- with (include "bcp.overrideVolumeMounts" $ctx | trim) }}{{ . | nindent 6 }}{{- end }}
volumes:
  {{- include "bcp.secretVolume" (dict "ctx" $ctx "component" "worker") | nindent 2 }}
  - name: work
    emptyDir:
      sizeLimit: {{ $w.workDir.sizeLimit }}
  - name: tmp
    emptyDir:
      sizeLimit: {{ $w.tmpSizeLimit }}
  - name: home
    emptyDir:
      sizeLimit: {{ $w.homeSizeLimit }}
  {{- with (include "bcp.overrideVolumes" $ctx | trim) }}{{ . | nindent 2 }}{{- end }}
{{- end -}}
