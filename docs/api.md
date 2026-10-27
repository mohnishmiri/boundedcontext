# API contract

Base path: `/api`. JSON in and out (`Content-Type: application/json`) unless stated. Times are RFC 3339 UTC.
Authentication is the `bcp_session` cookie (HttpOnly, Secure, SameSite=Lax) set by the OAuth callback.

**Mutating requests** (`POST`) must send `X-Requested-With: bcp` and an `Origin` equal to the public URL,
otherwise `403 csrf_rejected`. Request bodies are limited to 64 KiB.

## Errors

Every non-2xx response has this shape:

```ts
type ApiError = {
  error: {
    code: ErrorCode;            // stable, machine-readable
    message: string;            // human-readable, safe to display
    retryable: boolean;         // a retry of the same request may succeed
    details?: Record<string, unknown>;
    request_id: string;         // also in the X-Request-Id response header
  };
};

type ErrorCode =
  | "unauthenticated" | "csrf_rejected" | "forbidden" | "not_found" | "validation_failed"
  | "duplicate_job" | "conflict" | "rate_limited" | "payload_too_large"
  | "github_unavailable" | "github_auth_expired" | "github_rate_limited" | "repository_access_denied"
  | "app_not_installed" | "model_unavailable" | "model_locked_by_agent" | "copilot_unauthorized"
  | "agent_failed" | "agent_timeout" | "agent_unavailable"
  | "output_missing" | "output_empty" | "output_invalid" | "output_too_large" | "repository_too_large"
  | "base_branch_changed" | "branch_conflict" | "cancelled" | "internal";
```

| HTTP | Typical codes |
|---|---|
| 400 | `validation_failed` (with `details.fields: {name: message}`) |
| 401 | `unauthenticated`, `github_auth_expired` (sign in again) |
| 403 | `forbidden`, `csrf_rejected`, `repository_access_denied`, `generator_not_allowed`, `copilot_unauthorized` |
| 404 | `not_found`, `app_not_installed` |
| 409 | `duplicate_job` (`details.job_id`), `conflict`, `model_locked_by_agent` |
| 413 | `payload_too_large` |
| 422 | `model_unavailable`, `generator_invalid` |
| 429 | `rate_limited`, `github_rate_limited` (`Retry-After` header) |
| 502/503 | `github_unavailable`, `agent_unavailable` |

## Shared types

```ts
type JobStatus = "queued" | "running" | "succeeded" | "failed" | "cancelled";
type GenerationStage = "queued" | "preparing" | "fetching_repository" | "running_agent"
                     | "validating_output" | "completed";
type PublishStatus = "not_started" | "queued" | "running" | "succeeded" | "failed";
type PublishStage = "queued" | "checking_base" | "creating_commit" | "creating_branch"
                  | "creating_pull_request" | "checking_central_base" | "creating_central_commit"
                  | "creating_central_branch" | "creating_central_pull_request" | "completed";
type StepState = "pending" | "active" | "done" | "failed" | "skipped";

type Step = { key: string; label: string; state: StepState; started_at: string | null; finished_at: string | null };

type JobError = { code: ErrorCode; message: string; retryable: boolean };

type User = { id: string; login: string; name: string | null; avatar_url: string | null };

type JobSummary = {
  id: string;                 // uuid
  short_id: string;           // first 8 hex chars, used in branch names
  owner: string; repo: string; base_branch: string;
  model: string;
  status: JobStatus;
  stage: GenerationStage;
  publish_status: PublishStatus;
  pr_url: string | null;
  created_at: string; finished_at: string | null;
  requested_by: string;       // GitHub login
  batch_id: string | null;    // set when the job was created by `POST /batches`
};

type GeneratorSource = "builtin" | "curated" | "user";

type Job = JobSummary & {
  repository_url: string;     // https://github.com/{owner}/{repo}
  base_sha: string | null;    // resolved when the job starts
  agent: { id: string; version: string; placeholder: boolean };
  // Which generator produced this job; null only for jobs created before generators existed.
  generator: null | { id: string | null; name: string; source: GeneratorSource;
                      owner: string | null; repo: string | null; branch: string | null;
                      path: string | null; sha: string | null };   // sha = pinned revision that ran
  inputs: Record<string, string>;
  steps: Step[];              // generation steps, in order
  error: JobError | null;
  cancel_requested: boolean;
  can_cancel: boolean;
  can_publish: boolean;       // succeeded && publish not running/succeeded && user may push
  can_retry: boolean;
  started_at: string | null; updated_at: string;
  duration_ms: number | null;
  output: null | {
    primary_path: string;
    files: { path: string; change_type: "added" | "modified"; size_bytes: number }[];
    unexpected_changes: { path: string; change_type: "added" | "modified" | "deleted" }[]; // never committed
  };
  // Copilot usage from the CLI's --usage-output-file, summed over the job's agent runs (failed/timed-out runs
  // included); null until a run reports usage.
  usage: null | { runs: number; premium_requests: number; api_duration_ms: number;
                  input_tokens: number; output_tokens: number; cache_read_tokens: number;
                  cache_write_tokens: number; reasoning_tokens: number;
                  models: { model: string; requests: number; premium_requests: number; input_tokens: number;
                            output_tokens: number; cache_read_tokens: number; cache_write_tokens: number;
                            reasoning_tokens: number }[] };
  publish: {
    status: PublishStatus;
    stage: PublishStage | null;
    steps: Step[];
    title: string | null;
    central_available: boolean;       // BCP_CENTRAL_REPO is configured
    central_requested: boolean;       // the user opted into a central-repository PR
    central_repo: string | null;      // owner/repository
    central_branch: string | null;
    central_commit_sha: string | null;
    central_pr_number: number | null;
    central_pr_url: string | null;
    branch: string | null;
    commit_sha: string | null;
    commit_url: string | null;
    pr_number: number | null;
    pr_url: string | null;
    error: JobError | null;
    updated_at: string | null;
  };
};

type LogEntry = { seq: number; ts: string; level: "debug" | "info" | "warn" | "error"; stage: string; message: string };

type Page<T> = { items: T[]; page: number; per_page: number; has_more: boolean; total?: number };
```

## Endpoints

### Auth

| Method & path | Description |
|---|---|
| `GET /api/auth/github?return_to=/jobs/abc` | `302` to GitHub authorisation (state + PKCE). `return_to` must be a same-origin path. |
| `GET /api/auth/github/callback?code=…&state=…` | Exchanges the code, upserts the user, sets `bcp_session`, `302` to `return_to` (default `/`). On failure `302 /?auth_error=<code>`. |
| `GET /api/auth/me` | `200 { user: User }` or `401 unauthenticated`. |
| `POST /api/auth/logout` | Deletes the session. `204`. |

### GitHub browsing (user token; only what the user *and* the App can access)

`GET /api/github/organizations` → `200`
```ts
{ items: { installation_id: number; login: string; type: "Organization" | "User"; avatar_url: string | null }[];
  install_url: string }   // where to install the App on more accounts
```

`GET /api/github/repositories?owner=<login>&q=<substring>&page=1&per_page=30` (`per_page` ≤ 100) → `200 Page<Repository>`
```ts
type Repository = { id: number; owner: string; name: string; full_name: string; private: boolean;
  default_branch: string; html_url: string; description: string | null;
  permissions: { pull: boolean; push: boolean; admin: boolean }; updated_at: string | null };
```

`GET /api/github/repositories/{owner}/{repo}` → `200 Repository & { can_generate: boolean; can_publish: boolean; reason: string | null }`

`GET /api/github/repositories/{owner}/{repo}/branches?q=<substring>&page=1&per_page=50` → `200`
```ts
Page<{ name: string; commit_sha: string; protected: boolean }> & { default_branch: string }
```

`GET /api/copilot/quota` → `200` — the signed-in user's Copilot budget for the current billing period, read with
their own token (cached 60 s). Never fails for GitHub errors; returns `available: false` instead.
```ts
{ available: false; message: string }
| { available: true; login: string | null; plan: string | null;
    unit: "ai_credits" | "premium_requests";   // premium_requests on legacy billing
    unlimited: boolean; used: number | null; entitlement: number | null; remaining: number | null;
    percent_remaining: number | null; overage_permitted: boolean; overage_used: number;
    reset_date: string | null }
```

`/api/copilot/token` — an optional per-user token used **only** to run Copilot, so users who sign in with a classic
`ghp_` token (which Copilot rejects) still spend their own credits. A fine-grained token owned by the user's own
account with only the "Copilot Requests" permission needs no organisation approval. Stored encrypted.
- `GET` → `{ configured: boolean; sign_in_token_classic: boolean; shared: boolean; needed: boolean }`
- `POST { token }` → same shape. `400` for classic/invalid tokens, `403` if the token belongs to another account.
- `DELETE` → `204`.

Job creation fails with `403 copilot_unauthorized` when the user signed in with `ghp_`, has no Copilot token and the
worker has no shared token.

### Generators, agent and models

A **generator** is an agent bundle (`bundle.yaml` + agent files, see `agent-bundle/README.md`) that produces the
documentation. The built-in one ships with the worker image; operators can list *curated* generator repositories
and an allowlist of repositories users may add themselves (`config/generators.yaml`).

`GET /api/generators` → `200`
```ts
{ items: Generator[]; default_id: string /* "builtin" */; can_add: boolean }
type Generator = {
  id: string;                      // "builtin", a curated id, or a uuid for a saved (user) generator
  name: string; description: string | null;
  source: GeneratorSource;
  repository: { owner: string; repo: string; branch: string | null; path: string } | null;
  removable: boolean;              // true only for the caller's own saved generators
};
```

`POST /api/generators` `{ owner, repo, branch?, path?, name? }` → `201 Generator`. The repository must match the
operator allowlist (`403 generator_not_allowed`) and is actually fetched and loaded, so a saved generator is known
to work (`422 generator_invalid` explains what is wrong: no `bundle.yaml`, invalid manifest, outputs outside
documentation, MCP servers…). `path` is the directory holding `bundle.yaml` (default: repository root); `branch`
defaults to the repository's default branch. Re-adding the same repository/branch/path returns the existing entry.
`409 conflict` once the per-user cap (`max_user_generators`) is reached.

`DELETE /api/generators/{id}` → `204` (own saved generators only; `404` otherwise).

`GET /api/generators/{id}/agent` → `200` — what the form needs for that generator, resolved at the head of its
branch (`GET /api/agent` is the same for the built-in generator):
```ts
{ id: string; name: string; description: string; version: string; placeholder: boolean;
  output_paths: string[];               // where the agent writes, from the bundle manifest
  inputs: AgentInput[];                 // only inputs the existing workflow genuinely needs (often empty)
  locked_model: string | null;          // set when the agent file pins `model:` — selection is fixed
  lock_reason: string | null;
  resolved: { branch: string | null; sha: string | null } }   // null for the built-in generator
type AgentInput = { name: string; label: string; help: string | null; required: boolean;
  kind: "text" | "select" | "path"; choices: string[] | null; default: string | null; max_length: number };
```

`GET /api/models` → `200`
```ts
{ items: { id: string; name: string; vendor: string; description: string | null;
           multiplier: number | null; is_default: boolean }[];
  default_model: string;
  source: "copilot" | "catalog";        // "copilot": listed by GitHub Copilot for the caller's own token
  warning: string | null;               // set when the live lookup failed and the catalog is shown instead
  entitlement_note: string }
```
The list is fetched with the signed-in user's token (cached ~2 minutes per user), so it reflects that account's
plan and organisation policy. `auto` (Copilot chooses) is always offered. If the lookup fails the operator
catalog (`config/models.yaml`) is returned with `source: "catalog"` and a `warning`. Job creation validates the
model against the same list.

### Jobs

`POST /api/bounded-context/jobs`
```ts
// request
{ owner: string; repo: string; base_branch: string; model: string; inputs?: Record<string, string>;
  generator_id?: string /* default "builtin" */ }
```
→ `202 Job`. Validates fields, checks the user can read the repo (and warns via `can_publish` if they cannot
push), checks the model against the user's model list and the generator's model lock, rejects active duplicates
(`409 duplicate_job`; the same repo/branch/model may run once per generator at a time). Repository generators are
pinned to their branch head at request time (`job.generator.sha`).

`POST /api/bounded-context/batches` — one generation per selected repository
```ts
// request
{ generator_id: string; model: string; inputs?: Record<string, string>;
  repositories: { owner: string; repo: string; base_branch: string }[] }   // 1..25 (BCP_BATCH_MAX_REPOS)
// response 202
{ batch_id: string; jobs: JobSummary[];
  failures: { owner: string; repo: string; base_branch: string;
              error: { code: ErrorCode; message: string; details?: unknown } }[] }
```
Request-level problems (unknown generator, invalid model or inputs, empty/duplicate/oversized repository list)
fail the whole request with the usual error envelope before anything is queued. Each repository is then checked
on its own: the ones that pass are queued as independent jobs sharing `batch_id`; the rest are returned in
`failures` (e.g. `app_not_installed`, `repository_access_denied`, `duplicate_job`, `rate_limited`).

`GET /api/bounded-context/jobs?status=<JobStatus>&owner=&repo=&batch_id=&page=1&per_page=20` → `200 Page<JobSummary>`
(only the caller's jobs).

`GET /api/bounded-context/jobs/{job_id}` → `200 Job`

`GET /api/bounded-context/jobs/{job_id}/logs?after=<seq>&limit=200` → `200 { items: LogEntry[]; next_after: number }`

`GET /api/bounded-context/jobs/{job_id}/events` → `text/event-stream`
- `event: job` — `data: Job` (sent on connect and on every change)
- `event: log` — `data: LogEntry`
- `event: ping` — keep-alive every 15 s
The server closes the stream once the job and its publish sub-state are both terminal. Clients should
fall back to polling `GET /jobs/{id}` + `/logs` if SSE fails.

`GET /api/bounded-context/jobs/{job_id}/diff` → `200` (only when `status = succeeded`, else `409 conflict`)
```ts
{ base_sha: string;
  files: { path: string; change_type: "added" | "modified"; base_content: string | null;
           new_content: string; unified_diff: string; size_bytes: number; sha256: string }[];
  unexpected_changes: { path: string; change_type: "added" | "modified" | "deleted" }[] }
```

`GET /api/bounded-context/jobs/{job_id}/output` → `200 text/markdown` (primary output file, download)

`POST /api/bounded-context/jobs/{job_id}/pull-request` `{ title: string, publish_central?: boolean }` → `202 Job`
with `publish.status = "queued"`. The title is required (1–256 characters) and is used unchanged for the source PR
and, when `publish_central` is true, for the PR in `BCP_CENTRAL_REPO`; the organisation's PR-title policy validates
it on GitHub. Central files are written below `docs/<source-repo>/` on a new branch from the central repository's
default branch.
Idempotent: if already queued/running/succeeded returns `200 Job` unchanged. `409 conflict` if the job has not
succeeded or central publishing is not configured. `403 repository_access_denied` if the user lacks push permission
to the source repository, or to the central repository when it is selected.

`POST /api/bounded-context/jobs/{job_id}/cancel` → `202 Job` (`cancel_requested = true`, or `cancelled` if it
was still queued). `409 conflict` if already terminal.

`POST /api/bounded-context/jobs/{job_id}/retry` → `202 Job` — a **new** job with the same inputs and generator, at the
generator repository's current head (allowed for `failed`/`cancelled`).

### Operations (no auth, not routed through the public ingress except `/healthz`)

| Path | Port | Purpose |
|---|---|---|
| `GET /healthz` | 8080 (api), 8081 (worker) | Liveness: process is up. |
| `GET /readyz` | 8080 (api), 8081 (worker) | Readiness: DB reachable, migrations applied; worker also checks the Copilot CLI and agent bundle. |
| `GET /metrics` | 9090 (api), 9091 (worker) | Prometheus metrics. |

A machine-readable version lives in `docs/openapi.yaml`.
