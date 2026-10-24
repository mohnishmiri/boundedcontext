# Bounded Context Portal

A web portal that runs the organisation's existing **bounded-context agent** (the GitHub Copilot custom agent
developers use in VS Code) against any GitHub repository they can access, shows the generated documentation for
review, and opens a pull request — without anyone cloning the repository or opening VS Code.

```
Sign in with GitHub → pick a generator, organisation, one or more repositories (+ branch each) and a model → generate → review preview + diff → open PRs
```

- **Generic platform**: the *generator* (agent bundle) is selectable — the built-in one, operator-curated
  generator repositories, or a repository the user adds (if the operator allows it) — and a request can cover
  **many repositories** at once (`config/generators.yaml`, `docs/architecture.md`).
- **Reuses the real agent**: the agent files are run verbatim through **GitHub Copilot CLI** in non-interactive
  mode; nothing about the agent is re-implemented (`agent-bundle/`).
- **Models follow your token**: the model list comes from the signed-in user's own Copilot entitlement, with the
  operator catalog as a labelled fallback.
- **Each user's own Copilot entitlement**: runs use the signed-in user's GitHub App user token, so model access
  and policies are exactly what that user has in VS Code.
- **Least privilege**: users see only repositories they can access *and* where the GitHub App is installed;
  repository writes use a short-lived installation token scoped to one repository, only after re-checking the
  user's push permission; nothing is ever merged automatically.
- **Sandboxed agent**: no shell, URL or MCP tools; scrubbed environment; only allowlisted output paths can be
  committed; separate hardened pods (optionally one pod per run) on AKS.

> ⚠️ **The agent in `agent-bundle/` is a clearly labelled placeholder.** The real agent was not available when
> the platform was built (`docs/discovery.md`). Install it with `agent-bundle/README.md` — a file copy plus a
> manifest edit, no code change.

## Quick start (no credentials needed)

```bash
scripts/start.sh                  # builds on first run, picks free ports, waits until healthy, prints the URLs
open http://localhost:3000        # sign in through the mock GitHub as "octocat"
scripts/stop.sh                   # stop (keeps data); --wipe also deletes the database
```

Runs PostgreSQL, a mock GitHub, the API, a worker using a fake Copilot CLI, and the UI. See
[docs/local-development.md](docs/local-development.md).

## Repository layout

| Path | Contents |
|---|---|
| `backend/` | Rust workspace: `bcp-api` (Axum), `bcp-worker`, `bcp-agent` (bundle + Copilot CLI adapter + `bcp-agent-run`), `bcp-github`, `bcp-store` (SQLx + migrations), `bcp-core`, `bcp-services`; dev tools `mock-github`, `fake-copilot`, `integration-tests` |
| `frontend/` | React + TypeScript SPA (Vite, Tailwind, React Query), nginx image |
| `agent-bundle/` | The agent files (verbatim) and `bundle.yaml` (prompt, output paths, inputs, tool policy) |
| `config/models.yaml` | Operator-maintained model catalog (fallback for the live per-user list) |
| `config/generators.yaml` | Generator registry: curated generator repositories and the allowlist for user-added ones |
| `deploy/helm/` | Helm chart for AKS (Key Vault CSI, Workload Identity, KEDA, NetworkPolicies, app routing) |
| `deploy/observability/` | Prometheus alert rules |
| `docs/` | Design and operations documentation |

## Documentation

| Document | For |
|---|---|
| [docs/discovery.md](docs/discovery.md) | What was verified about the agent and Copilot CLI, and what still needs confirming |
| [docs/architecture.md](docs/architecture.md) | Components, agent adapter contract, identity model, job lifecycle, data model |
| [docs/api.md](docs/api.md) · [docs/openapi.yaml](docs/openapi.yaml) | HTTP API contract |
| [docs/security.md](docs/security.md) | Threat model and controls |
| [docs/azure-setup.md](docs/azure-setup.md) | Provisioning Azure, the GitHub App, deployment and the Copilot smoke test |
| [docs/operations.md](docs/operations.md) | Runbook: metrics, scaling, rotation, upgrades, troubleshooting |
| [docs/local-development.md](docs/local-development.md) | Running and testing locally |
| [agent-bundle/README.md](agent-bundle/README.md) | Installing the real agent |

## Status

| Area | State |
|---|---|
| Backend (API, worker, Copilot CLI adapter, GitHub integration, queue) | Implemented; unit, integration and end-to-end tests |
| Frontend | Implemented; component and flow tests |
| Helm chart, images, CI | Implemented; chart lints and renders for dev/staging/prod |
| Real bounded-context agent | **Pending** — placeholder installed |
| Live validation against GitHub Copilot with a GitHub App user token | **Pending** — smoke test in `docs/azure-setup.md` §7 |
