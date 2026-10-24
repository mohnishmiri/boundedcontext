# Azure and GitHub setup

End-to-end provisioning for one environment (repeat per dev / staging / prod with different names). Commands use
the Azure CLI; translate them to Bicep/Terraform for your landing zone as needed. Replace every `<placeholder>`.

```bash
ENV=prod
LOC=westeurope
RG=rg-bcp-$ENV
AKS=aks-bcp-$ENV
ACR=bcpacr                       # shared across environments
KV=kv-bcp-$ENV
PG=pg-bcp-$ENV
NS=bcp
HOST=bcp.example.com             # public host name of the portal
```

## 1. Prerequisites

- Azure CLI ≥ 2.60, `kubectl`, `helm` ≥ 3.14, rights to create resource groups, role assignments and federated
  credentials.
- A GitHub organisation where you can register and install a GitHub App, and **GitHub Copilot Business or
  Enterprise** with **Copilot CLI** allowed by organisation policy for the users of the portal.
- A DNS zone for `$HOST` and a TLS certificate (Key Vault certificate or cert-manager).

## 2. Core resources

```bash
az group create -n $RG -l $LOC

# Network: one VNet with an AKS subnet and a delegated subnet for PostgreSQL private access.
az network vnet create -g $RG -n vnet-bcp-$ENV --address-prefixes 10.40.0.0/16 \
  --subnet-name snet-aks --subnet-prefixes 10.40.0.0/20
az network vnet subnet create -g $RG --vnet-name vnet-bcp-$ENV -n snet-pg \
  --address-prefixes 10.40.16.0/24 --delegations Microsoft.DBforPostgreSQL/flexibleServers
AKS_SUBNET=$(az network vnet subnet show -g $RG --vnet-name vnet-bcp-$ENV -n snet-aks --query id -o tsv)

az acr create -g $RG -n $ACR --sku Standard   # once, shared

# AKS with workload identity, Key Vault CSI driver, KEDA, app routing (managed NGINX), managed Prometheus and
# Azure CNI powered by Cilium (needed for FQDN egress policies, §4).
az aks create -g $RG -n $AKS --location $LOC \
  --kubernetes-version 1.31 \
  --network-plugin azure --network-plugin-mode overlay --network-dataplane cilium \
  --vnet-subnet-id $AKS_SUBNET \
  --enable-oidc-issuer --enable-workload-identity \
  --enable-addons azure-keyvault-secrets-provider --enable-secret-rotation --rotation-poll-interval 5m \
  --enable-keda \
  --enable-app-routing \
  --enable-azure-monitor-metrics \
  --attach-acr $ACR \
  --node-count 3 --zones 1 2 3 --node-vm-size Standard_D4ds_v5

az aks get-credentials -g $RG -n $AKS
kubectl create namespace $NS
OIDC_ISSUER=$(az aks show -g $RG -n $AKS --query oidcIssuerProfile.issuerUrl -o tsv)
```

Optional: a dedicated node pool for workers (`--node-taints bcp/worker=true:NoSchedule`) and the matching
`worker.tolerations` / `worker.nodeSelector` in values, so agent runs never share nodes with the API.

## 3. Release pipeline identity (GitHub Actions → ACR)

`.github/workflows/release.yml` pushes images with OpenID Connect; no registry password is stored.

```bash
az identity create -g $RG -n id-bcp-release
REL_CLIENT=$(az identity show -g $RG -n id-bcp-release --query clientId -o tsv)
REL_PRINCIPAL=$(az identity show -g $RG -n id-bcp-release --query principalId -o tsv)
az role assignment create --assignee-object-id $REL_PRINCIPAL --assignee-principal-type ServicePrincipal \
  --role AcrPush --scope $(az acr show -n $ACR --query id -o tsv)
az identity federated-credential create -g $RG --identity-name id-bcp-release -n github-tags \
  --issuer https://token.actions.githubusercontent.com \
  --subject "repo:<org>/<repo>:environment:release" --audiences api://AzureADTokenExchange
```

Repository variables: `AZURE_CLIENT_ID=$REL_CLIENT`, `AZURE_TENANT_ID`, `AZURE_SUBSCRIPTION_ID`, `ACR_NAME=$ACR`.
Protect the `release` environment with required reviewers.

## 4. Networking and egress

| Component | Needs to reach |
|---|---|
| frontend | nothing (served via ingress) |
| api | PostgreSQL; `api.github.com`, `github.com` (OAuth token exchange) |
| worker | PostgreSQL; `api.github.com`, `codeload.github.com` (tarballs); GitHub Copilot service hosts |

The chart installs default-deny NetworkPolicies with these allows (`deploy/helm/.../networkpolicy.yaml`).
Plain NetworkPolicy can only allow "HTTPS to public addresses"; with Cilium set `networkPolicy.cilium.enabled=true`
to restrict egress to the FQDNs in `values.yaml` (`githubFqdns`, `copilotFqdnPatterns`). **Verify the Copilot hosts
against GitHub's current "Copilot allowlist reference"** — for subscription-based routing use
`*.business.githubcopilot.com` or `*.enterprise.githubcopilot.com`. If egress goes through a corporate proxy,
add `HTTPS_PROXY`/`NO_PROXY` via `config.extraEnv`; the agent runtime receives them (and `NODE_EXTRA_CA_CERTS`).

For GitHub Enterprise Server set `config.github.apiUrl=https://<ghes>/api/v3` and `webUrl=https://<ghes>`, and
adjust the FQDN lists. Mount the GHES CA and set `SSL_CERT_FILE` / `NODE_EXTRA_CA_CERTS` if it is private.

## 5. Data, secrets and identities

### PostgreSQL Flexible Server

```bash
az postgres flexible-server create -g $RG -n $PG --location $LOC \
  --version 16 --tier GeneralPurpose --sku-name Standard_D2ds_v5 --storage-size 64 \
  --high-availability ZoneRedundant --backup-retention 14 \
  --vnet vnet-bcp-$ENV --subnet snet-pg \
  --private-dns-zone $PG.private.postgres.database.azure.com \
  --admin-user bcpadmin --admin-password '<generated>'
az postgres flexible-server db create -g $RG -s $PG -d bcp
```

Create an application role with least privilege (the migration hook needs DDL on its own schema only):

```sql
CREATE ROLE bcp_app LOGIN PASSWORD '<generated>';
GRANT CONNECT ON DATABASE bcp TO bcp_app;
GRANT USAGE, CREATE ON SCHEMA public TO bcp_app;
```

Connection string stored in Key Vault (TLS verified against the public CA chain Azure uses):
`postgres://bcp_app:<password>@$PG.postgres.database.azure.com:5432/bcp?sslmode=verify-full`

### Key Vault

```bash
az keyvault create -g $RG -n $KV --location $LOC --enable-rbac-authorization true \
  --enable-purge-protection true --retention-days 90
az keyvault secret set --vault-name $KV -n database-url --value '<connection string>'
az keyvault secret set --vault-name $KV -n github-client-secret --value '<GitHub App client secret>'
az keyvault secret set --vault-name $KV -n github-app-private-key --file <app>.private-key.pem
az keyvault secret set --vault-name $KV -n token-encryption-key --value "$(openssl rand -base64 32)"
# TLS certificate for the ingress (app routing add-on reads it from Key Vault)
az keyvault certificate import --vault-name $KV -n bcp-tls --file bcp-tls.pfx
```

### Workload identities

One user-assigned identity per component, federated to its Kubernetes service account:

```bash
for c in api worker; do
  az identity create -g $RG -n id-bcp-$c-$ENV
  PRINCIPAL=$(az identity show -g $RG -n id-bcp-$c-$ENV --query principalId -o tsv)
  az role assignment create --assignee-object-id $PRINCIPAL --assignee-principal-type ServicePrincipal \
    --role "Key Vault Secrets User" --scope $(az keyvault show -n $KV --query id -o tsv)
  az identity federated-credential create -g $RG --identity-name id-bcp-$c-$ENV -n bcp-$c \
    --issuer $OIDC_ISSUER --subject system:serviceaccount:$NS:bcp-$c --audiences api://AzureADTokenExchange
done
# The migration hook uses the api identity under its own service account name.
az identity federated-credential create -g $RG --identity-name id-bcp-api-$ENV -n bcp-migrate \
  --issuer $OIDC_ISSUER --subject system:serviceaccount:$NS:bcp-migrate --audiences api://AzureADTokenExchange

# App routing add-on: allow it to read the TLS certificate.
az aks approuting update -g $RG -n $AKS --enable-kv --attach-kv $(az keyvault show -n $KV --query id -o tsv)
```

Record the two client ids for `serviceAccounts.api.clientId` / `serviceAccounts.worker.clientId`.

**Rotation.** Secrets are read at pod start. After rotating a Key Vault secret, wait for the CSI poll interval and
restart: `kubectl -n $NS rollout restart deploy/bcp-api deploy/bcp-worker` (see `docs/operations.md`). Rotating
`token-encryption-key` invalidates stored GitHub tokens (users simply sign in again).

## 6. GitHub App

Register one App per environment (organisation settings → Developer settings → GitHub Apps → New):

| Setting | Value |
|---|---|
| Homepage URL | `https://$HOST` |
| Callback URL | `https://$HOST/api/auth/github/callback` |
| Expire user authorization tokens | **Enabled** (the portal refreshes tokens server-side) |
| Request user authorization during installation | Optional |
| Enable Device Flow | Disabled |
| Webhook | **Inactive** (not used) |
| Repository permissions | Metadata: Read-only · Contents: **Read and write** · Pull requests: **Read and write** |
| Organization / account permissions | None, plus whatever Copilot requires for user-to-server tokens (§7) |
| Where can this App be installed | Only on this account |

Then: generate a client secret and a private key (store both in Key Vault, §5), note the App id, client id and
slug for `config.github.*`, and **install the App** on the organisation for the repositories the portal may use
(all or selected). Users only ever see repositories that are both covered by the installation and accessible to
them.

## 7. Deploy and smoke test

### Deploy

Build and push images (or let `release.yml` do it on a tag), then:

```bash
helm upgrade --install bcp deploy/helm/bounded-context-portal -n $NS \
  -f deploy/helm/bounded-context-portal/values-$ENV.yaml \
  --set api.image.tag=<version> --set worker.image.tag=<version> --set frontend.image.tag=<version> \
  --wait --timeout 10m
kubectl -n $NS get pods
```

The pre-install/pre-upgrade hook runs `bcp-api migrate`. Readiness: `/readyz` on the api checks the database and
migrations; on the worker it also checks the Copilot CLI (`copilot --version`).

### Smoke test — Copilot with a GitHub App user token

GitHub documents that Copilot CLI accepts `ghu_` user-to-server tokens via `COPILOT_GITHUB_TOKEN`, but not which
App permission (if any) is required. Validate it once per App:

1. Confirm the pinned CLI supports the flags the adapter uses:
   ```bash
   kubectl -n $NS exec deploy/bcp-worker -- copilot --help | grep -E -- '--(agent|model|allow-tool|deny-tool|disable-mcp-server|no-custom-instructions|no-ask-user|share|log-dir)'
   ```
   (In `once` mode run a temporary pod from the worker image instead.) If a flag is missing, pin a CLI version
   that has it (`COPILOT_CLI_VERSION` build arg) or adjust `bundle.yaml`.
2. Sign in to `https://$HOST` as a user with a Copilot seat and start a job on a small test repository with
   model `auto`.
3. Expected: the job reaches *Running bounded-context agent* and completes. If it fails with
   `copilot_unauthorized`, check (a) the user has an assigned Copilot seat, (b) organisation policy allows
   Copilot CLI, (c) the GitHub App has any Copilot-related account permission GitHub offers for your plan, then
   have the user sign in again so the new permission is granted. `model_unavailable` means the model is not
   enabled for that user/org — adjust `config/models.yaml`.
4. Open the pull request from the review screen and confirm the branch, commit trailer and PR body.

### Install the real agent

Until `agent-bundle/` contains the real agent, the UI shows a placeholder warning. Follow
`agent-bundle/README.md`, rebuild the images (or mount the bundle as a ConfigMap via `agentBundle.configMap`),
and redeploy.
