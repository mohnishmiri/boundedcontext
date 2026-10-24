#!/usr/bin/env bash
# Starts the local Bounded Context Portal stack (docker compose): PostgreSQL, mock GitHub, API, worker with the
# fake Copilot CLI, and the UI. Picks free host ports when the defaults are taken, waits until every component
# is healthy and prints the URLs. Safe to run again: if the stack is already up it just prints the URLs.
#
#   scripts/start.sh            start (builds images the first time, which takes several minutes)
#   scripts/start.sh --build    rebuild images from the current sources, then (re)start
#   scripts/start.sh --fresh    delete the database and dev secrets first, then start
#   scripts/start.sh --logs     follow API and worker logs after starting
#
# Ports can be forced with BCP_UI_PORT, BCP_API_PORT, BCP_PG_PORT, BCP_MOCK_PORT.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

BUILD=0
FRESH=0
FOLLOW=0
WAIT_SECS=180

usage() { sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; }

while [ $# -gt 0 ]; do
  case "$1" in
    -b|--build) BUILD=1 ;;
    --fresh) FRESH=1 ;;
    -f|--logs) FOLLOW=1 ;;
    --timeout) shift; WAIT_SECS="${1:?--timeout needs seconds}" ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

if [ -t 1 ]; then
  BOLD=$'\033[1m'; DIM=$'\033[2m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; RED=$'\033[31m'; RESET=$'\033[0m'
else
  BOLD=; DIM=; GREEN=; YELLOW=; RED=; RESET=
fi
info() { printf '%s\n' "${DIM}›${RESET} $*"; }
warn() { printf '%s\n' "${YELLOW}!${RESET} $*" >&2; }
die()  { printf '%s\n' "${RED}✗${RESET} $*" >&2; exit 1; }

# ---- prerequisites -------------------------------------------------------------------------------------------
command -v docker >/dev/null 2>&1 || die "Docker is not installed."
docker info >/dev/null 2>&1 || die "Docker is not running. Start Docker Desktop and try again."
docker compose version >/dev/null 2>&1 || die "The Docker Compose plugin is missing (docker compose)."

compose() { docker compose "$@"; }

# Something is listening on 127.0.0.1:<port>?
port_in_use() { (exec 3<>"/dev/tcp/127.0.0.1/$1") >/dev/null 2>&1; }

# First free port: the preferred one, else the first free one in [from, from+50).
pick_port() {
  local preferred=$1 from=$2 p
  if ! port_in_use "$preferred"; then echo "$preferred"; return; fi
  for ((p = from; p < from + 50; p++)); do
    if ! port_in_use "$p"; then echo "$p"; return; fi
  done
  return 1
}

# Host port a running service publishes for <container port>, e.g. "55432".
published_port() { compose port "$1" "$2" 2>/dev/null | sed 's/.*://' | head -n1; }

running() { [ -n "$(compose ps --status running -q "$1" 2>/dev/null)" ]; }

# ---- ports ---------------------------------------------------------------------------------------------------
if [ "$FRESH" = 1 ]; then
  info "Removing containers, database and dev secrets (--fresh)"
  compose down -v --remove-orphans >/dev/null 2>&1 || true
fi

if running frontend && running api && running mock-github && running postgres; then
  # Keep the ports of the running stack so URLs and sessions stay valid.
  UI_PORT=$(published_port frontend 8080)
  API_PORT=$(published_port api 8080)
  PG_PORT=$(published_port postgres 5432)
  MOCK_PORT=$(published_port mock-github 9000)
  ALREADY_UP=1
else
  ALREADY_UP=0
  UI_PORT=${BCP_UI_PORT:-$(pick_port 3000 3001)} || die "No free port for the UI (tried 3000, 3001-3050)."
  API_PORT=${BCP_API_PORT:-$(pick_port 8080 58080)} || die "No free port for the API (tried 8080, 58080-58129)."
  PG_PORT=${BCP_PG_PORT:-$(pick_port 5432 55432)} || die "No free port for PostgreSQL (tried 5432, 55432-55481)."
  MOCK_PORT=${BCP_MOCK_PORT:-$(pick_port 9000 9001)} || die "No free port for the mock GitHub (tried 9000, 9001-9050)."
  # Only mention ports that were moved automatically (not ones forced through BCP_*_PORT).
  for pair in "UI:$UI_PORT:3000:${BCP_UI_PORT:-}" "API:$API_PORT:8080:${BCP_API_PORT:-}" \
              "PostgreSQL:$PG_PORT:5432:${BCP_PG_PORT:-}" "mock GitHub:$MOCK_PORT:9000:${BCP_MOCK_PORT:-}"; do
    IFS=: read -r name chosen default forced <<<"$pair"
    if [ -z "$forced" ] && [ "$chosen" != "$default" ]; then
      warn "Port $default is in use; $name will use $chosen."
    fi
  done
fi

export BCP_UI_PORT=$UI_PORT BCP_API_PORT=$API_PORT BCP_PG_PORT=$PG_PORT BCP_MOCK_PORT=$MOCK_PORT
export BCP_PUBLIC_URL="http://localhost:$UI_PORT"

# ---- start ---------------------------------------------------------------------------------------------------
if [ "$ALREADY_UP" = 1 ] && [ "$BUILD" = 0 ]; then
  info "The stack is already running."
else
  if [ "$BUILD" = 1 ]; then
    info "Building images from the current sources…"
    compose build
  elif [ -z "$(docker image ls -q bounded-context-portal-api 2>/dev/null)" ]; then
    info "First start: building images (Rust release build, takes several minutes)…"
  fi
  info "Starting containers…"
  compose up -d --remove-orphans

  # ---- wait for health ---------------------------------------------------------------------------------------
  deadline=$((SECONDS + WAIT_SECS))
  wait_for() {
    local label=$1; shift
    printf '%s' "${DIM}›${RESET} Waiting for $label "
    until "$@" >/dev/null 2>&1; do
      if [ $SECONDS -ge $deadline ]; then
        printf '%s\n' "${RED}timed out${RESET}"
        warn "Recent logs:"
        compose logs --tail 30 api worker frontend >&2 || true
        die "$label did not become ready within ${WAIT_SECS}s (see logs above, or: docker compose logs)."
      fi
      printf '.'
      sleep 1
    done
    printf '%s\n' "${GREEN}ready${RESET}"
  }
  wait_for "API" curl -fsS "http://localhost:$API_PORT/readyz"
  wait_for "worker" compose exec -T worker node -e \
    "fetch('http://127.0.0.1:8081/readyz').then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))"
  wait_for "UI" curl -fsS "http://localhost:$UI_PORT/healthz"
fi

# ---- summary -------------------------------------------------------------------------------------------------
row() { printf '  %-13s %-42s %s\n' "$1" "$2" "${3:-}"; }
printf '\n%s\n\n' "${GREEN}${BOLD}Bounded Context Portal is running${RESET}"
row "Portal" "http://localhost:$UI_PORT" "open in your browser"
row "Mock GitHub" "http://localhost:$MOCK_PORT" "sign in as octocat (push access) or viewer (read-only)"
row "API" "http://localhost:$API_PORT" "/readyz, /api/…"
row "PostgreSQL" "postgres://bcp:bcp@localhost:$PG_PORT/bcp"
cat <<EOF

  Agent runtime: fake Copilot CLI (test double); agent bundle: placeholder.
  Logs:  docker compose logs -f api worker
  Stop:  scripts/stop.sh (keeps data)  ·  scripts/stop.sh --wipe (deletes data)
EOF

if [ "$FOLLOW" = 1 ]; then
  echo
  compose logs -f api worker
fi
