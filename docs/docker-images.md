# Docker image build

How to build, verify and push the three production images to JFrog. Deploying them is covered in
[helm-deployment.md](helm-deployment.md).

| Image | Source | Base | Contents | Ports |
|---|---|---|---|---|
| `bcp-api` | `backend/Dockerfile`, target `api`, context = repo root | `gcr.io/distroless/cc-debian12:nonroot` | `bcp-api` binary, `agent-bundle/`, `generators/`, `config/models.yaml`, `config/generators.yaml` | 8080 http, 9090 metrics |
| `bcp-worker` | `backend/Dockerfile`, target `worker`, context = repo root | `node:22-bookworm-slim` | `bcp-worker`, `bcp-agent-run`, GitHub Copilot CLI (pinned), `tini`, `agent-bundle/`, `generators/`, `config/models.yaml` | 8081 http, 9091 metrics |
| `bcp-frontend` | `frontend/Dockerfile`, context = `frontend/` | `nginxinc/nginx-unprivileged:1.27-alpine` | Built SPA + nginx config template | 8080 |

All images run as non-root (api/worker uid 10001, frontend uid 101) and work with a read-only root filesystem.
The `worker-dev` and `dev-tools` targets (fake Copilot, mock GitHub) are for local Docker Compose only — **never
push them**.

Registry (same repository path, different host):

| Side | Host | Example |
|---|---|---|
| `docker login` / build / push | `artifact.it.att.com` (no port) | `artifact.it.att.com/apm0014313-dkr-attcc-stage/com.att.attcc/bcp-api:1.0.0` |
| AKS pull (`global.imageRegistry`, pull secret) | `artifact.it.att.com:22609` | `artifact.it.att.com:22609/apm0014313-dkr-attcc-stage/com.att.attcc/bcp-api:1.0.0` |

## 1. Prerequisites

- Docker Desktop (or Docker Engine ≥ 24 with BuildKit, the default). No local Rust, Node or Cargo needed.
- Network access to Docker Hub, `gcr.io`, `crates.io`, `registry.npmjs.org` (directly or through the corporate
  proxy).
- The corporate root CA bundle when traffic goes through TLS inspection, e.g. `C:/binary/requests-ca-bundle.pem`.
- A JFrog account (or service account) with deploy rights on `apm0014313-dkr-attcc-stage`.

## 2. Corporate CA

Builds download crates, npm packages and the Copilot CLI, so behind TLS inspection the CA must be trusted at build
time. Set `BCP_CA_CERT_FILE` (in `.env` or the environment). It is passed as a BuildKit **secret** (never stored
in an image layer) and appended to each image's trust store
(`/etc/ssl/certs/ca-certificates-extra.crt`, used through `SSL_CERT_FILE` / `NODE_EXTRA_CA_CERTS`), so the
running api and worker also trust the corporate proxy when calling GitHub and Copilot.

Leave it unset when the build host and the cluster have direct internet access.

## 3. Build and push (script)

```bash
docker login artifact.it.att.com
scripts/push-images.sh 1.0.0              # build all three, push, print the Helm --set flags
scripts/push-images.sh 1.0.0 --no-push    # build and tag only
REGISTRY=artifact.it.att.com/<other-path> scripts/push-images.sh 1.0.0   # different repository path
```

The script builds from the current working tree, picks up `BCP_CA_CERT_FILE` from `.env`, and tags
`$REGISTRY/bcp-api:<tag>`, `$REGISTRY/bcp-worker:<tag>` and `$REGISTRY/bcp-frontend:<tag>` with
`REGISTRY=artifact.it.att.com/apm0014313-dkr-attcc-stage/com.att.attcc` by default.

If you change the path, set the same path (with `:22609`) in Helm `global.imageRegistry`.

## 4. Build and push (manual)

Run from the repository root (Git Bash on Windows works; use forward slashes in paths):

```bash
REG=artifact.it.att.com/apm0014313-dkr-attcc-stage/com.att.attcc     # push side: no port
TAG=1.0.0
CA="--secret id=corporate_ca,src=C:/binary/requests-ca-bundle.pem --build-arg CORPORATE_CA_CERT_ENABLED=1"   # omit without a corporate CA

docker build $CA -f backend/Dockerfile --target api    -t $REG/bcp-api:$TAG .
docker build $CA -f backend/Dockerfile --target worker -t $REG/bcp-worker:$TAG .
docker build $CA -t $REG/bcp-frontend:$TAG frontend

docker push $REG/bcp-api:$TAG
docker push $REG/bcp-worker:$TAG
docker push $REG/bcp-frontend:$TAG
```

### Build arguments

| Argument | Default | Image | Purpose |
|---|---|---|---|
| `RUST_VERSION` | `1.90` | api, worker | Rust toolchain for the build stage |
| `COPILOT_CLI_VERSION` | `1.0.103` | worker | `@github/copilot` npm version (pinned; `COPILOT_AUTO_UPDATE=false` at run time) |
| `NODE_VERSION` | `22` | worker | Node.js runtime for the Copilot CLI |
| `CORPORATE_CA_CERT_ENABLED` | empty | all | `1` = append the `corporate_ca` secret to the trust store |

## 5. Verify before pushing

```bash
docker image ls "$REG/bcp-*"
docker run --rm --entrypoint copilot $REG/bcp-worker:$TAG --version        # expect the pinned CLI version
docker run --rm --entrypoint ls $REG/bcp-worker:$TAG /opt/agent-bundle /opt/generators
docker run --rm -d -p 18080:8080 --name fe-check -e API_UPSTREAM=http://127.0.0.1:9 $REG/bcp-frontend:$TAG \
  && sleep 2 && curl -fsS http://127.0.0.1:18080/healthz; docker rm -f fe-check
```

The local Docker Compose stack (`scripts/start.sh --build`) builds from the same Dockerfiles and is the quickest
end-to-end check before a release.

## 6. Tagging and promotion

- Use an immutable version per release (e.g. `1.4.0`, or `1.4.0-<git-sha>`); never reuse or overwrite a tag and
  do not deploy `latest`.
- Build once, deploy that tag to **Perf**, verify, then deploy the **same tag** to **Prod** — never rebuild for
  Prod.
- For the strongest guarantee pin digests instead of tags: after pushing,
  `docker inspect --format '{{index .RepoDigests 0}}' $REG/bcp-api:$TAG` and pass
  `--set api.image.digest=sha256:...` (digest wins over tag; same for worker and frontend).

## 7. When to rebuild

| Change | Rebuild |
|---|---|
| Rust code (`backend/`) | api + worker |
| Frontend code (`frontend/`) | frontend |
| `agent-bundle/`, `generators/`, `config/models.yaml` | api + worker (both bake them in) |
| `config/generators.yaml` | api |
| Copilot CLI upgrade (`COPILOT_CLI_VERSION`) | worker (see [operations.md](operations.md#upgrade-copilot-cli)) |
| Base image security patches | all (rebuild regularly, e.g. monthly) |

`models.yaml`, `generators.yaml` and the agent bundle can also be overridden at deploy time through Helm values
(`models.content`, `generators.content`, `agentBundle.configMap`) without rebuilding.

## 8. Troubleshooting

| Symptom | Fix |
|---|---|
| `x509: certificate signed by unknown authority` / `unable to get local issuer certificate` during build | Set `BCP_CA_CERT_FILE` (or the `--secret ... --build-arg CORPORATE_CA_CERT_ENABLED=1` pair) |
| `failed to solve: ... secret corporate_ca` | Path in `src=` is wrong, or BuildKit is disabled (`DOCKER_BUILDKIT=1`) |
| `denied` / `unauthorized` on push | `docker login artifact.it.att.com` again (no port); check deploy permission on the repository |
| First build very slow | Expected (full Rust release build); later builds reuse the BuildKit cargo cache |
| Pods show `ImagePullBackOff` | See [helm-deployment.md](helm-deployment.md#9-troubleshooting) (pull secret / path) |
