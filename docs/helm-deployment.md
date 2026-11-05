# Helm deployment (AKS)

How to deploy the Bounded Context Portal to AKS with the chart in `deploy/helm/bounded-context-portal`. Images are
built and pushed first — see [docker-images.md](docker-images.md). Azure resource provisioning in general is in
[azure-setup.md](azure-setup.md); day-2 operations are in [operations.md](operations.md).

| Environment | URL | Values file | Order |
|---|---|---|---|
| **Perf** | https://ragaflow-boundedcontext.test.att.com | `values-perf.yaml` | 1st — deploy and verify here |
| **Prod** | https://ragaflow-boundedcontext.web.att.com | `values-prod.yaml` | 2nd — promote the **same image tag** |

Both pull images from `artifact.it.att.com:22609/apm0014313-dkr-attcc-stage/com.att.attcc` (pushed without the
port to `artifact.it.att.com/...`, see [docker-images.md](docker-images.md)) with the pull secret
`regcred`, production sizing
(api 3–8 replicas, frontend 2–6, one worker pod per agent run up to 20), PAT sign-in, admin `mz7819_ATT` and
central repo `ATT-DP1/apm0014313-ragaflow-ai-context-docs`.

## 1. What the chart deploys

| Resource | Purpose |
|---|---|
| `bcp-api` Deployment, Service, HPA, PDB | Rust API (`/api`), sessions, job queue |
| `bcp-frontend` Deployment, Service, HPA, PDB | nginx serving the UI (the ingress sends `/api` straight to the api) |
| `bcp-worker` KEDA `ScaledJob` + `TriggerAuthentication` | One Kubernetes Job per queued task (`worker.mode: once`), runs Copilot CLI |
| Migrations | Run by the api at start-up (`BCP_RUN_MIGRATIONS=true`, forced in `akv2k8s` mode; serialised by a database lock) |
| `AzureKeyVaultSecret` per Key Vault object (+ TLS certificate) | akv2k8s syncs `database-url` → Secret `bcp-database-url`, `token-encryption-key` → `bcp-token-encryption-key`, certificate → `bcp-tls`; pods mount them as files, KEDA reads `bcp-database-url` |
| ServiceAccounts `bcp-api`, `bcp-worker` | No Azure identity needed in `akv2k8s` mode |
| ConfigMaps | Non-secret configuration (`BCP_*` env vars) |
| Ingress | Host + TLS, `/api` → api, `/` → frontend, long timeouts for live log streams (SSE) |
| NetworkPolicies | Default deny; allow ingress controller, DNS, PostgreSQL, public HTTPS (GitHub/Copilot) |
| PodMonitor | Prometheus scraping (Azure Monitor managed Prometheus) |

Secrets use [akv2k8s](https://akv2k8s.io) (`secrets.mode: akv2k8s` in both values files). The chart also supports
`csi` (Secrets Store CSI driver + Workload Identity, with a pre-install migration hook) — see
[azure-setup.md](azure-setup.md#workload-identities) if you switch to it.

Resource names use the release name as prefix; the commands below assume release `bcp` in namespace `bounded-context`.

## 2. Cluster prerequisites (once per cluster)

| Requirement | Check |
|---|---|
| AKS ≥ 1.29, `kubectl`, `helm` ≥ 3.14 | `kubectl version`, `helm version` |
| akv2k8s controller | `kubectl get crd azurekeyvaultsecrets.spv.no` and `kubectl -n akv2k8s get pods` (install: `helm repo add spv-charts https://charts.spvapi.no && helm upgrade --install akv2k8s spv-charts/akv2k8s -n akv2k8s --create-namespace`) |
| KEDA add-on | `kubectl get crd scaledjobs.keda.sh` |
| App routing add-on (managed NGINX) | `kubectl get ingressclass webapprouting.kubernetes.azure.com` |
| Azure Monitor managed Prometheus (PodMonitor CRD) — optional | `kubectl get crd podmonitors.azmonitoring.coreos.com`; without it the chart skips the PodMonitors (pods keep `prometheus.io/*` annotations) |

Different ingress controller? Set `ingress.className` and `networkPolicy.ingressController.namespaceSelector`
(namespace of the controller pods).

## 3. Inputs per environment

The values files leave cluster-specific identifiers empty on purpose; the chart refuses to render until they are
set. They are identifiers, not secrets — put them in the values file once known, or pass them with `--set`.

| Value | Perf | Prod | Where it comes from |
|---|---|---|---|
| `secrets.keyVault.name` | REQUIRED | REQUIRED | Key Vault holding the app secrets (§4.3) |
| `ingress.tls.akv2k8sCertName` **or** `keyVaultCertUri` **or** `secretName` | REQUIRED | REQUIRED | TLS certificate for the host (§4.5) |
| `networkPolicy.postgres.cidrs` | `135.170.109.97/32` (set; ignored while `networkPolicy.enabled: false`) | REQUIRED | IP of the environment's PostgreSQL server (§4.6) |
| `api/worker/frontend.image.tag` | per release | same tag as Perf | [docker-images.md](docker-images.md) |

Fill them in `values-perf.local.yaml` / `values-prod.local.yaml` (next to the other values files in the chart
directory) and pass that file after the environment file. Filled-in example:

```yaml
secrets:
  keyVault: { name: kv-ragaflow-perf }
ingress:
  tls: { akv2k8sCertName: ragaflow-boundedcontext }
```

## 4. One-time setup per environment

```bash
ENV=perf                                         # or prod
NS=bounded-context
HOST=ragaflow-boundedcontext.test.att.com        # prod: ragaflow-boundedcontext.web.att.com
KV=<key vault name>
RG=<resource group>
AKS=<aks cluster>
```

### 4.1 Namespace

```bash
kubectl create namespace $NS
```

### 4.2 JFrog pull secret

Use a JFrog service account / identity token with read access to the docker repository:

```bash
kubectl -n $NS create secret docker-registry regcred \
  --docker-server=artifact.it.att.com:22609 \
  --docker-username='<jfrog user>' --docker-password='<jfrog identity token>'
```

### 4.3 Key Vault secrets

Only two secrets are needed with the current configuration (PAT sign-in, per-user Copilot tokens):

| Secret | Value |
|---|---|
| `database-url` | `postgres://<user>:<password>@<server>.postgres.database.azure.com:5432/bcp?sslmode=require` — percent-encode special characters in the password (`@` → `%40`, `#` → `%23`, `%` → `%25`) |
| `token-encryption-key` | `openssl rand -base64 32` — a **different** key per environment; never change it once in use (stored tokens become unreadable and users must re-enter them) |

```bash
az keyvault secret set --vault-name $KV -n database-url --value '<connection string>'
az keyvault secret set --vault-name $KV -n token-encryption-key --value "$(openssl rand -base64 32)"
```

Optional secrets are only requested when the feature is enabled: `github-client-secret` (`config.github.clientId`
set — OAuth sign-in), `github-app-private-key` (`config.github.appId` set), `copilot-github-token`
(`secrets.copilotSharedToken=true`), `smtp-password` (`config.smtp.host` and `config.smtp.user` set — completion
emails; `az keyvault secret set --vault-name $KV -n smtp-password --value '<password>'`).

Use a least-privilege database role per environment (see [azure-setup.md](azure-setup.md#postgresql-flexible-server)),
not the server admin. Perf and Prod must use separate databases.

### 4.4 Key Vault access for akv2k8s

The akv2k8s controller reads Key Vault with the cluster's identity (by default the AKS kubelet managed identity
from `/etc/kubernetes/azure.json`, unless your akv2k8s install is configured with its own identity). Give that
identity read access to secrets and certificates:

```bash
KV_ID=$(az keyvault show -n $KV --query id -o tsv)
KUBELET_ID=$(az aks show -g $RG -n $AKS --query identityProfile.kubeletidentity.objectId -o tsv)
# RBAC-enabled Key Vault
az role assignment create --assignee-object-id $KUBELET_ID --assignee-principal-type ServicePrincipal \
  --role "Key Vault Secrets User" --scope $KV_ID
az role assignment create --assignee-object-id $KUBELET_ID --assignee-principal-type ServicePrincipal \
  --role "Key Vault Certificate User" --scope $KV_ID       # only for ingress.tls.akv2k8sCertName
# Access-policy Key Vault instead
az keyvault set-policy -n $KV --object-id $KUBELET_ID --secret-permissions get --certificate-permissions get
```

The pods themselves need no Azure identity (`serviceAccounts.*.clientId` stay empty). After the deploy, check the
sync with `kubectl -n $NS get azurekeyvaultsecrets` (status *Synced*) and `kubectl -n $NS get secret bcp-database-url`.
Pods wait in `ContainerCreating` until their Secrets exist.

Rotation: akv2k8s updates the Kubernetes Secret when the Key Vault version changes; restart the api to pick it up
(`kubectl -n $NS rollout restart deploy/bcp-api`; new worker Jobs read it automatically).

### 4.5 TLS certificate

Recommended with akv2k8s: import the certificate for `$HOST` into Key Vault and set
`ingress.tls.akv2k8sCertName` to its name — the chart creates an `AzureKeyVaultSecret` that syncs it into the TLS
secret `bcp-tls`:

```bash
az keyvault certificate import --vault-name $KV -n ragaflow-boundedcontext --file <cert>.pfx
# ingress.tls.akv2k8sCertName: ragaflow-boundedcontext
```

Alternatives: the app routing add-on reads it directly (`ingress.tls.keyVaultCertUri`, needs
`az aks approuting update -g $RG -n $AKS --enable-kv --attach-kv $KV_ID`), or create a TLS secret yourself and set
`ingress.tls.secretName=ragaflow-tls`:

```bash
kubectl -n $NS create secret tls ragaflow-tls --cert=<fullchain>.pem --key=<key>.pem
```

### 4.6 Database network access

- **Network policy:** `networkPolicy.postgres.cidrs` must contain the PostgreSQL server address (the pods are only
  allowed to reach those IPs on 5432). Resolve it from inside the cluster:
  `kubectl run dns-check --rm -it --image=busybox:1.36 --restart=Never -- nslookup <server>.postgres.database.azure.com`.
  Perf: `attcc-eastus2-prf1-db-psqlfs` → `135.170.109.97`.
- **PostgreSQL firewall** (public access): allow the AKS outbound IP(s):
  `az aks show -g $RG -n $AKS --query "networkProfile.loadBalancerProfile.effectiveOutboundIPs[].id" -o tsv`
  (or the NAT gateway IP), then `az postgres flexible-server firewall-rule create ...`.
- **Connections:** api replicas × `databaseMaxConnections` (8 × 10) + worker pods × 4 (20 × 4) ≈ 160 at full
  scale. Make sure the server's `max_connections` allows it, or lower `worker.scaledJob.maxReplicaCount` /
  `config.databaseMaxConnections`.

### 4.7 DNS

After the first deploy, point `$HOST` at the ingress address:

```bash
kubectl -n $NS get ingress bcp -o jsonpath='{.status.loadBalancer.ingress[0].ip}'
```

### 4.8 Outbound access

The api and worker need HTTPS to `api.github.com`, `github.com`, `codeload.github.com` and the GitHub Copilot hosts
(`*.githubcopilot.com`, `copilot-proxy.githubusercontent.com`). If the cluster egresses through a proxy, set
`config.proxy.http` / `config.proxy.https` (Perf: `http://proxy.conexus.svc.local:3128`) — the chart sets
`HTTP(S)_PROXY` / `NO_PROXY` for the api and worker (the worker passes them to the Copilot CLI) and, with
`networkPolicy.proxy.enabled`, allows egress to the proxy namespace. If the proxy inspects TLS, build the images
with the corporate CA ([docker-images.md](docker-images.md#2-corporate-ca)).

**GitHub Enterprise IP allow list.** The `ATT-EMU` enterprise only accepts API calls from allow-listed IPs, so the
cluster's outbound public IP(s) must be added (enterprise settings → *Authentication security* → *IP allow list*)
before anyone can sign in:

```bash
az aks show -g $RG -n $AKS --query "networkProfile.outboundType" -o tsv     # loadBalancer | userDefinedRouting | managedNATGateway
az aks show -g $RG -n $AKS --query "networkProfile.loadBalancerProfile.effectiveOutboundIPs[].id" -o tsv \
  | xargs -I{} az network public-ip show --ids {} --query ipAddress -o tsv
kubectl run egress-ip --rm -it --image=curlimages/curl --restart=Never -- curl -s https://api.ipify.org   # simplest
```

With `userDefinedRouting` (traffic through a firewall/proxy) allow-list the firewall's public IP instead.

## 5. Validate before deploying

Run from the repository root (or set `CHART` to an absolute path). Every variable must be set — an empty `$NS`
makes `-n` swallow the next flag (`Error: expected at most two arguments`):

```bash
CHART=deploy/helm/bounded-context-portal         # from inside the chart directory: CHART=.
NS=bounded-context
TAG=1.0.0
IMG="--set api.image.tag=$TAG --set worker.image.tag=$TAG --set frontend.image.tag=$TAG"

helm lint $CHART -f $CHART/values-perf.yaml -f $CHART/values-perf.local.yaml $IMG
helm template bcp $CHART -n $NS -f $CHART/values-perf.yaml -f $CHART/values-perf.local.yaml $IMG > /tmp/bcp-perf.yaml
helm upgrade --install bcp $CHART -n $NS -f $CHART/values-perf.yaml -f $CHART/values-perf.local.yaml $IMG --dry-run=server
```

A missing input fails with a clear message, e.g. `secrets.keyVault.name is required for secrets.mode=akv2k8s`,
`ingress.tls.secretName, ingress.tls.keyVaultCertUri or ingress.tls.akv2k8sCertName is required` or
`networkPolicy.postgres.cidrs is required`.

## 6. Deploy to Perf

```bash
helm upgrade --install bcp $CHART -n $NS \
  -f $CHART/values-perf.yaml -f $CHART/values-perf.local.yaml $IMG \
  --wait --timeout 15m
```

What happens: the `AzureKeyVaultSecret`s are created and akv2k8s syncs the Kubernetes Secrets; the api pods start
once they exist, apply the database migrations (first replica wins the lock) and become ready; then the frontend,
worker ScaledJob, ingress and policies follow. Watch it:

```bash
kubectl -n $NS get azurekeyvaultsecrets                # all Synced
kubectl -n $NS get pods -w
kubectl -n $NS logs deploy/bcp-api --tail=50           # migrations, then "listening"
kubectl -n $NS get deploy,hpa,scaledjob,ingress
```

### Verify Perf

```bash
curl -fsS https://ragaflow-boundedcontext.test.att.com/healthz                        # frontend
kubectl -n $NS port-forward deploy/bcp-api 18080:8080 & sleep 2; curl -fsS http://127.0.0.1:18080/readyz; kill %1
kubectl -n $NS get secret bcp-database-url bcp-token-encryption-key bcp-tls           # synced by akv2k8s
```

Then a functional smoke test in the browser:

1. Open https://ragaflow-boundedcontext.test.att.com and sign in with a GitHub personal access token.
2. Save a fine-grained Copilot token with the **"Copilot Requests"** user permission (the portal validates it).
3. Start a job on a small repository: a worker Job pod appears (`kubectl -n $NS get pods -l app.kubernetes.io/component=worker`),
   live logs stream, and the job reaches *Review*.
4. Publish and confirm the pull request in the central repository, including reviewers.
5. As `mz7819_ATT`, check the admin view shows all users' jobs.

## 7. Promote to Prod

Same chart, **same image tag** that passed Perf, prod inputs:

```bash
helm upgrade --install bcp $CHART -n $NS --kube-context <prod-context> \
  -f $CHART/values-prod.yaml -f $CHART/values-prod.local.yaml $IMG \
  --wait --timeout 15m
```

Repeat §6 verification against https://ragaflow-boundedcontext.web.att.com. Users re-enter their tokens in Prod
(separate database and encryption key).

## 8. Upgrade, rollback, uninstall

```bash
# Upgrade: push a new tag, then re-run the §6/§7 command with the new $TAG
helm -n $NS history bcp
helm -n $NS rollback bcp <REVISION> --wait
```

Migrations are forward-only; a rollback does not revert the schema (new migrations are written to stay
compatible with the previous release). Running worker Jobs are not interrupted by an upgrade.

Uninstall (the database and Key Vault are untouched; the synced Secrets are removed with their
`AzureKeyVaultSecret`s):

```bash
helm -n $NS uninstall bcp
```

## 9. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| `ImagePullBackOff`, `unauthorized` | `regcred` missing in the `bounded-context` namespace or wrong credentials (server must be `artifact.it.att.com:22609`); wrong path in `global.imageRegistry` (must match the push path, plus `:22609`) |
| Pod stuck `ContainerCreating`, event `secret "bcp-database-url" not found` | akv2k8s has not synced: `kubectl -n bounded-context describe azurekeyvaultsecret bcp-database-url` and the akv2k8s controller logs (`kubectl -n akv2k8s logs deploy/akv2k8s-controller`). Usual causes: wrong `secrets.keyVault.name`, Key Vault secret missing, or the controller identity lacks *get* on secrets (§4.4) |
| `no matches for kind "AzureKeyVaultSecret"` on install | akv2k8s (its CRDs) is not installed in the cluster (§2) |
| api crash-loops with `running migrations` / database errors | `database-url` wrong (encoding, `sslmode=require`), PostgreSQL firewall does not allow the AKS egress IP, or `networkPolicy.postgres.cidrs` does not match the server IP |
| api not ready (`/readyz` 503) | Database unreachable or migrations not applied — check `kubectl -n bounded-context logs deploy/bcp-api` |
| Jobs stay *Queued*, no worker pods | KEDA cannot read the DB: secret `bcp-database-url` missing or the DB unreachable from KEDA — `kubectl -n bounded-context describe scaledjob bcp-worker` |
| Sign-in / jobs fail with `403 ... enterprise has an IP allow list enabled` | The cluster's egress IP is not in the GitHub Enterprise (`ATT-EMU`) IP allow list — add the AKS outbound IP(s) there (§4.8) |
| Worker logs `missing Copilot Requests permission` / 401 | The user's Copilot token lacks the "Copilot Requests" permission or Copilot seat — the user re-creates the token |
| GitHub/Copilot calls fail with TLS errors | Egress proxy with TLS inspection: images need the corporate CA and `HTTPS_PROXY` in `config.extraEnv` |
| Browser 404/TLS error | DNS not pointing to the ingress, or the certificate does not cover the host |
| Live logs stop after ~60 s | Ingress controller other than app routing NGINX: raise its read timeout (`ingress.sseTimeoutSeconds` covers NGINX) |

## 10. `.env` → Helm mapping

Every variable used locally has a Helm equivalent:

| `.env` (local) | Helm |
|---|---|
| `BCP_PUBLIC_URL` | `config.publicUrl` |
| `BCP_DATABASE_URL` / `BCP_EXTERNAL_DATABASE_URL` | Key Vault `database-url` → Secret `bcp-database-url` (mounted, `BCP_DATABASE_URL_FILE`) |
| `BCP_TOKEN_ENCRYPTION_KEY` | Key Vault `token-encryption-key` → Secret `bcp-token-encryption-key` (`BCP_TOKEN_ENCRYPTION_KEY_FILE`) |
| `BCP_DATABASE_MAX_CONNECTIONS` | `config.databaseMaxConnections` |
| `BCP_ADMIN_LOGINS` | `config.adminLogins` |
| `BCP_CENTRAL_REPO` | `config.extraEnv.BCP_CENTRAL_REPO` |
| `BCP_GITHUB_API_URL`, `_WEB_URL`, `_APP_ID`, `_APP_SLUG`, `_CLIENT_ID` | `config.github.*` |
| `BCP_GITHUB_CLIENT_SECRET`, `BCP_GITHUB_PRIVATE_KEY` | Key Vault (only when `clientId` / `appId` set) |
| `BCP_COPILOT_GITHUB_TOKEN` | Key Vault `copilot-github-token` (only with `secrets.copilotSharedToken=true`) |
| `SMTP_HOST`, `SMTP_PORT`, `SMTP_USER`, `SMTP_FROM_ADDRESS`, `SMTP_FROM_NAME`, `SMTP_USE_TLS`, `BCP_NOTIFY_EMAIL_DOMAIN` | `config.smtp.*` (worker; egress via `networkPolicy.smtp.cidrs`) |
| `SMTP_PASSWORD` | Key Vault `smtp-password` → Secret `bcp-smtp-password` (`SMTP_PASSWORD_FILE`, worker only) |
| `BCP_LOG_FORMAT`, `RUST_LOG`, `BCP_MODELS_SHOW_UNVERIFIED` | `config.logFormat`, `config.rustLog`, `config.modelsShowUnverified` |
| `BCP_AGENT_BUNDLE_DIR`, `BCP_GENERATORS_FILE`, `BCP_MODELS_FILE` | `config.agentBundleDir`, `config.generatorsFile`, `config.modelsFile` |
| `BCP_SESSION_TTL_HOURS`, `BCP_COOKIE_SECURE`, `BCP_RATE_LIMIT_PER_MINUTE`, `BCP_JOB_CREATE_LIMIT_PER_HOUR`, `BCP_RUN_MIGRATIONS`, `BCP_JOB_RETENTION_DAYS` | `api.env.*` |
| `BCP_MAX_JOB_TIMEOUT_SECS`, `BCP_MAX_REPO_*`, `BCP_TASK_LEASE_SECS`, `BCP_*_MAX_ATTEMPTS`, `BCP_SHUTDOWN_GRACE_SECS`, `BCP_IDLE_EXIT_SECS`, `BCP_MAX_LOG_LINES` | `worker.env.*` |
| `BCP_WORKER_CONCURRENCY`, `BCP_WORKER_MODE` | `worker.concurrency`, `worker.mode` |
| `BCP_API_LISTEN`, `BCP_WORKER_LISTEN`, `*_METRICS_LISTEN`, `BCP_WORK_DIR` | Fixed by the chart |
| `BCP_COPILOT_BIN` | Set in the worker image |
| `BCP_CA_CERT_FILE` | Build time only ([docker-images.md](docker-images.md#2-corporate-ca)) |
