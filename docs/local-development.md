# Local development

## Option A — everything in Docker, no credentials (recommended first run)

```bash
scripts/start.sh          # first run builds the images (several minutes)
scripts/stop.sh           # stop; data is kept
```

| Script option | Effect |
|---|---|
| `scripts/start.sh --build` | Rebuild images from the current sources, then restart |
| `scripts/start.sh --fresh` | Delete the database and dev secrets, then start |
| `scripts/start.sh --logs` | Follow API and worker logs after starting |
| `scripts/stop.sh --wipe` | Also delete the database and dev secrets |
| `scripts/stop.sh --all` | `--wipe` plus remove the locally built images |

`start.sh` uses ports 3000 (UI), 8080 (API), 5432 (PostgreSQL) and 9000 (mock GitHub) when free and otherwise
picks free alternatives (it prints which); force them with `BCP_UI_PORT`, `BCP_API_PORT`, `BCP_PG_PORT`,
`BCP_MOCK_PORT`. Plain `docker compose up --build` also works with the default ports.

If Docker image builds fail with `UNABLE_TO_GET_ISSUER_CERT_LOCALLY` while downloading npm packages on a
TLS-inspected corporate network, set `BCP_CA_CERT_FILE` in `.env` to the PEM root CA path and rebuild with
`scripts/start.sh --build`. The script passes it as a BuildKit secret rather than a build argument. The Rust,
worker, and frontend build stages add it to their trusted CA bundles, and the API and worker images keep it so
they can reach GitHub at runtime; TLS verification remains enabled. Direct `docker compose` builds also use the
path from
`BCP_CA_CERT_FILE` in `.env`.

Open the portal URL it prints (normally <http://localhost:3000>) and sign in. The **mock GitHub** (http://localhost:9000) shows an account chooser:

| Account | Can see | Push access |
|---|---|---|
| `octocat` | `acme/*` (App installed) and `octocat/hello-world` | yes (read-only on `acme/readonly-docs`) |
| `viewer` | `acme/shop`, `acme/readonly-docs` | no — can generate, cannot publish |
| `outsider` | nothing | — |

The worker runs **fake-copilot**, a test double for GitHub Copilot CLI that writes a bounded-context document
from the repository's directory layout. Seeded repositories exercise different paths:

| Repository | Behaviour |
|---|---|
| `acme/shop` | Normal run (branches `main`, `develop`, `feature/checkout-v2`) |
| `acme/legacy-monolith` | Agent also edits `README.md` → shown as an *unexpected change*, never committed |
| `acme/broken-agent` | Agent exits with an error → `agent_failed` |
| `acme/not-installed` | App not installed → `app_not_installed` |
| `acme/svc-01` … `svc-35` | Pagination |

A repository can choose fake-copilot's behaviour with a `.fake-copilot-mode` file: `ok`, `slow` (≈30 s, try
cancelling), `hang`, `fail`, `unauthorized`, `model_unavailable`, `no_output`, `empty`, `unexpected`.

Mock control endpoints (handy while developing the UI):

```bash
curl -X POST localhost:9000/_mock/reset                     # reseed
curl localhost:9000/_mock/state | jq .pulls                  # pull requests "opened" by the portal
curl -X POST localhost:9000/_mock/repos/acme/shop/advance \
     -H 'content-type: application/json' \
     -d '{"branch":"main","path":"docs/bounded-context.md","content":"# edited"}'   # provoke base_branch_changed
curl -X POST localhost:9000/_mock/faults -H 'content-type: application/json' \
     -d '{"endpoint":"create_pull","status":502,"count":1}'                          # transient GitHub failure
```

## Option B — frontend with hot reload

Keep the compose stack running (it provides API, worker, database and mock), then:

```bash
cd frontend && npm install && npm run dev        # http://localhost:5173, proxies /api to localhost:8080
```

The API checks `Origin` on mutations, so start the API with the dev origin:
`BCP_PUBLIC_URL=http://localhost:5173 docker compose up -d api` (sign-in callbacks then also land on 5173).

## Option C — backend natively

Requires Rust ≥ 1.85 and `openssl` (tests generate a throwaway App key). Start only the infrastructure:

```bash
docker compose up -d postgres mock-github dev-secrets
cp .env.example .env    # then point it at the local services, e.g.
#   BCP_GITHUB_API_URL=http://localhost:9000  BCP_GITHUB_WEB_URL=http://localhost:9000
#   BCP_GITHUB_PRIVATE_KEY_FILE / BCP_TOKEN_ENCRYPTION_KEY_FILE -> copies of the dev-secrets volume files
#   BCP_COPILOT_BIN=$(pwd)/backend/target/debug/fake-copilot
cd backend
cargo build
set -a; . ../.env; set +a
cargo run -p bcp-api -- serve &
cargo run -p bcp-worker -- run
```

## Tests

```bash
cd backend
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
# Integration and end-to-end tests need PostgreSQL (each test creates its own database):
export BCP_TEST_DATABASE_URL=postgres://bcp:bcp@localhost:5432/bcp
cargo test --workspace

cd ../frontend && npm run typecheck && npm run lint && npm test
```

Test layers:

| Where | What |
|---|---|
| `crates/*/src` unit tests | validators, crypto, redaction, templates, bundle parsing, tarball safety, output classification, CLI argv |
| `crates/bcp-github/tests` | GitHub client against wiremock (retries, rate limits, pagination, OAuth errors) |
| `crates/bcp-agent/tests` | Copilot adapter process handling with shell-script stand-ins (env scrubbing, redaction, timeout kills the process group, cancellation) |
| `crates/bcp-store/tests` | queue semantics on PostgreSQL (SKIP LOCKED, leases, reaper, duplicates, retention) |
| `tools/integration-tests` | API + worker + PostgreSQL + mock GitHub end to end (OAuth, CSRF, browsing, generate, review, publish, conflicts, cancellation, SSE) |
| `frontend/src/**/*.test.tsx` | UI flows with MSW |

## Generators, batches and models locally

- The mock GitHub seeds generator repositories under `genhub/` (`lite`, `monorepo` with a bundle in
  `generators/ctx`, plus invalid ones) that `octocat` can read. They are not installed on any App, so they never
  show in the repository picker. The default `config/generators.yaml` allows `mohnishmiri/*` only; to try
  user-added generators against the mock, add `genhub/*` to `allow:` (or mount your own file) and restart the API.
- The model list is catalog-only unless the API is pointed at a Copilot API. With the mock set
  `BCP_COPILOT_API_URL=http://mock-github:9000/copilot` to get the live, per-user path (octocat and viewer see
  different lists). Against real github.com the default is `https://api.githubcopilot.com`.
- The integration tests use a fixture bundle (`backend/tools/integration-tests/fixtures/bundle`) and their own
  generator registry, so they do not depend on `agent-bundle/` or `config/generators.yaml`.

## Running against real GitHub and Copilot

1. Register a personal **development GitHub App** (see `docs/azure-setup.md` §6) with callback
   `http://localhost:3000/api/auth/github/callback` and install it on a test organisation.
2. In `.env`: real `BCP_GITHUB_*` values, `BCP_GITHUB_API_URL=https://api.github.com`,
   `BCP_GITHUB_WEB_URL=https://github.com`, remove `BCP_GITHUB_OAUTH_URL`.
3. Build the real worker image (`--target worker`), which contains Copilot CLI, or install `@github/copilot`
   locally and set `BCP_COPILOT_BIN=copilot`.

To try the agent alone, without the portal, use `bcp-agent-run` (see `agent-bundle/README.md`).
