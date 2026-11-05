#!/usr/bin/env bash
# Builds the production images (bcp-api, bcp-worker with the real Copilot CLI, bcp-frontend) and pushes them to
# JFrog, ready for deploy/helm/bounded-context-portal (values-perf.yaml, then values-prod.yaml).
#
#   scripts/push-images.sh <tag>            build and push, e.g. scripts/push-images.sh 1.0.0
#   scripts/push-images.sh <tag> --no-push  build and tag only
#
# Images are pushed to REGISTRY (default artifact.it.att.com/apm0014313-dkr-attcc-stage/com.att.attcc, no port);
# log in first with `docker login artifact.it.att.com`. AKS pulls the same path through artifact.it.att.com:22609
# (global.imageRegistry in the chart). The corporate CA (BCP_CA_CERT_FILE, from .env or the environment) is added
# to the images' trust store when set.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TAG="${1:-}"
PUSH=1
[ "${2:-}" = "--no-push" ] && PUSH=0
if [ -z "$TAG" ] || [ "$TAG" = "-h" ] || [ "$TAG" = "--help" ]; then
  sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 2
fi
case "$TAG" in *[!A-Za-z0-9._-]*) echo "Invalid tag: $TAG" >&2; exit 2 ;; esac

REGISTRY="${REGISTRY:-artifact.it.att.com/apm0014313-dkr-attcc-stage/com.att.attcc}"
if [ -z "${BCP_CA_CERT_FILE:-}" ] && [ -f .env ]; then
  BCP_CA_CERT_FILE="$(sed -n 's/^BCP_CA_CERT_FILE=//p' .env | tail -1 | tr -d '"'"'"'\r')"
fi

CA_ARGS=()
if [ -n "${BCP_CA_CERT_FILE:-}" ]; then
  [ -f "$BCP_CA_CERT_FILE" ] || { echo "BCP_CA_CERT_FILE not found: $BCP_CA_CERT_FILE" >&2; exit 1; }
  CA_ARGS=(--secret "id=corporate_ca,src=$BCP_CA_CERT_FILE" --build-arg CORPORATE_CA_CERT_ENABLED=1)
fi

export DOCKER_BUILDKIT=1
build() { # <image> <docker build args...>
  local ref="$REGISTRY/$1:$TAG"; shift
  echo "› building $ref"
  docker build "${CA_ARGS[@]}" -t "$ref" "$@"
  IMAGES+=("$ref")
}

IMAGES=()
build bcp-api      -f backend/Dockerfile --target api .
build bcp-worker   -f backend/Dockerfile --target worker .
build bcp-frontend frontend --build-arg "APP_VERSION=$TAG"

if [ "$PUSH" = 1 ]; then
  for ref in "${IMAGES[@]}"; do
    echo "› pushing $ref"
    docker push "$ref"
  done
fi

printf '\nHelm image flags:\n  --set api.image.tag=%s --set worker.image.tag=%s --set frontend.image.tag=%s\n' "$TAG" "$TAG" "$TAG"
