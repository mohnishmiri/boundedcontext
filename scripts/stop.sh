#!/usr/bin/env bash
# Stops the local Bounded Context Portal stack started by scripts/start.sh.
#
#   scripts/stop.sh           stop and remove the containers; keep the database and dev secrets
#   scripts/stop.sh --wipe    also delete the database and dev secrets (users, jobs, generated keys)
#   scripts/stop.sh --all     --wipe, and also remove the locally built images

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

WIPE=0
IMAGES=0

usage() { sed -n '2,6p' "$0" | sed 's/^# \{0,1\}//'; }

while [ $# -gt 0 ]; do
  case "$1" in
    -v|--wipe) WIPE=1 ;;
    --all) WIPE=1; IMAGES=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

if [ -t 1 ]; then GREEN=$'\033[32m'; DIM=$'\033[2m'; RESET=$'\033[0m'; else GREEN=; DIM=; RESET=; fi

command -v docker >/dev/null 2>&1 || { echo "Docker is not installed." >&2; exit 1; }
docker info >/dev/null 2>&1 || { echo "Docker is not running; nothing to stop." >&2; exit 0; }

if [ -z "$(docker compose ps -a -q 2>/dev/null)" ] && [ "$WIPE" = 0 ]; then
  echo "The stack is not running."
  exit 0
fi

args=(down --remove-orphans)
[ "$WIPE" = 1 ] && args+=(--volumes)
[ "$IMAGES" = 1 ] && args+=(--rmi local)

printf '%s\n' "${DIM}›${RESET} docker compose ${args[*]}"
docker compose "${args[@]}"

if [ "$WIPE" = 1 ]; then
  printf '%s\n' "${GREEN}Stopped.${RESET} Database and dev secrets deleted."
else
  printf '%s\n' "${GREEN}Stopped.${RESET} Data kept; start again with scripts/start.sh (or wipe it with scripts/stop.sh --wipe)."
fi
