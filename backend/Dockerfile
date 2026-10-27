# syntax=docker/dockerfile:1.7
#
# Images for the Bounded Context Portal backend. Build from the repository root:
#
#   docker build -f backend/Dockerfile --target api    -t bcp-api .
#   docker build -f backend/Dockerfile --target worker -t bcp-worker .
#
# Dev-only targets (never push to a production registry):
#   worker-dev  worker image using the fake Copilot CLI   (docker compose)
#   dev-tools   mock GitHub + fake Copilot + openssl       (docker compose)

ARG RUST_VERSION=1.90
# GitHub Copilot CLI (npm @github/copilot). Pinned: COPILOT_AUTO_UPDATE=false keeps it there at run time.
# Bump deliberately and re-run the smoke test in docs/azure-setup.md §7.
ARG COPILOT_CLI_VERSION=1.0.103
ARG NODE_VERSION=22

# ---------------------------------------------------------------------------------------------------------
FROM rust:${RUST_VERSION}-bookworm AS build
WORKDIR /src/backend
# Part of the cache key, so toggling the corporate CA rebuilds the trust store (secret contents are not).
ARG CORPORATE_CA_CERT_ENABLED
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates-extra.crt \
    CARGO_HTTP_CAINFO=/etc/ssl/certs/ca-certificates-extra.crt
COPY backend/ ./
RUN --mount=type=secret,id=corporate_ca,required=false \
    cp /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates-extra.crt \
 && if [ "$CORPORATE_CA_CERT_ENABLED" = "1" ]; then \
      cat /run/secrets/corporate_ca >> /etc/ssl/certs/ca-certificates-extra.crt; \
     fi
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/backend/target,sharing=locked \
    cargo build --release --locked \
        -p bcp-api -p bcp-worker -p bcp-agent -p mock-github -p fake-copilot \
 && mkdir -p /out \
 && cp target/release/bcp-api target/release/bcp-worker target/release/bcp-agent-run \
       target/release/mock-github target/release/fake-copilot /out/

# ---------------------------------------------------------------------------------------------------------
# API: static-ish Rust binary on distroless (glibc + CA certificates, no shell, non-root).
FROM gcr.io/distroless/cc-debian12:nonroot AS api
COPY --from=build /etc/ssl/certs/ca-certificates-extra.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=build /out/bcp-api /usr/local/bin/bcp-api
# The API reads the agent bundle (GET /api/agent, input validation, model lock) and the model catalog.
COPY agent-bundle/ /opt/agent-bundle/
COPY config/models.yaml /etc/bcp/models.yaml
COPY config/generators.yaml /etc/bcp/generators.yaml
ENV BCP_AGENT_BUNDLE_DIR=/opt/agent-bundle \
    BCP_MODELS_FILE=/etc/bcp/models.yaml \
    BCP_GENERATORS_FILE=/etc/bcp/generators.yaml \
    BCP_WORK_DIR=/tmp/bcp-work \
    BCP_LOG_FORMAT=json
USER 10001:10001
EXPOSE 8080 9090
ENTRYPOINT ["/usr/local/bin/bcp-api"]
CMD ["serve"]

# ---------------------------------------------------------------------------------------------------------
# Worker: Node.js runtime for GitHub Copilot CLI + the Rust worker. tini is PID 1 so orphaned agent
# processes killed by the worker are reaped instead of accumulating as zombies.
FROM node:${NODE_VERSION}-bookworm-slim AS worker
ARG COPILOT_CLI_VERSION
ARG CORPORATE_CA_CERT_ENABLED
ENV NODE_EXTRA_CA_CERTS=/etc/ssl/certs/ca-certificates-extra.crt \
    SSL_CERT_FILE=/etc/ssl/certs/ca-certificates-extra.crt
RUN --mount=type=secret,id=corporate_ca,required=false \
    apt-get update \
 && apt-get install -y --no-install-recommends tini ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && cp /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates-extra.crt \
 && if [ "$CORPORATE_CA_CERT_ENABLED" = "1" ]; then \
      cat /run/secrets/corporate_ca >> /etc/ssl/certs/ca-certificates-extra.crt; \
    fi \
 && npm install -g --omit=dev --no-audit --no-fund "@github/copilot@${COPILOT_CLI_VERSION}" \
 && npm cache clean --force \
 && groupadd --gid 10001 worker \
 && useradd --uid 10001 --gid 10001 --home-dir /home/worker --create-home --shell /usr/sbin/nologin worker \
 && mkdir -p /work && chown 10001:10001 /work
COPY --from=build /out/bcp-worker /out/bcp-agent-run /usr/local/bin/
COPY agent-bundle/ /opt/agent-bundle/
COPY config/models.yaml /etc/bcp/models.yaml
ENV BCP_AGENT_BUNDLE_DIR=/opt/agent-bundle \
    BCP_MODELS_FILE=/etc/bcp/models.yaml \
    BCP_COPILOT_BIN=/usr/local/bin/copilot \
    BCP_WORK_DIR=/work \
    BCP_LOG_FORMAT=json \
    COPILOT_AUTO_UPDATE=false \
    HOME=/home/worker \
    NODE_ENV=production
USER 10001:10001
EXPOSE 8081 9091
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/bcp-worker"]
CMD ["run"]

# ---------------------------------------------------------------------------------------------------------
# Local development only: same worker, but the agent runtime is the fake Copilot CLI.
FROM worker AS worker-dev
COPY --from=build /out/fake-copilot /usr/local/bin/fake-copilot
ENV BCP_COPILOT_BIN=/usr/local/bin/fake-copilot

# ---------------------------------------------------------------------------------------------------------
# Local development only: mock GitHub, fake Copilot and openssl (dev secret generation).
FROM debian:bookworm-slim AS dev-tools
RUN apt-get update && apt-get install -y --no-install-recommends openssl ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /out/mock-github /out/fake-copilot /usr/local/bin/
USER 65532:65532
EXPOSE 9000
CMD ["mock-github"]
