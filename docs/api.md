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
| 403 | `forbidden`, `csrf_rejected`, `repository_access_denied`, `copilot_unauthorized` |
| 404 | `not_found`, `app_not_installed` |
| 409 | `duplicate_job` (`details.job_id`), `conflict`, `model_locked_by_agent` |
| 413 | `payload_too_large` |
| 422 | `model_unavailable` |
| 429 | `rate_limited`, `github_rate_limited` (`Retry-After` header) |
| 502/503 | `github_unavailable`, `agent_unavailable` |

## Shared types

```ts
type JobStatus = "queued" | "running" | "succeeded" | "failed" | "cancelled";
type GenerationStage = "queued" | "preparing" | "fetching_repository" | "running_agent"
                     | "validating_output" | "completed";
type PublishStatus = "not_started" | "queued" | "running" | "succeeded" | "failed";
type PublishStage = "queued" | "checking_base" | "creating_commit" | "creating_branch"
                  | "creating_pull_request" | "completed";
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
};

type Job = JobSummary & {
  repository_url: string;     // https://github.com/{owner}/{repo}
  base_sha: string | null;    // resolved when the job starts
  agent: { id: string; version: string; placeholder: boolean };
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
  publish: {
    status: PublishStatus;
    stage: PublishStage | null;
    steps: Step[];
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

### Agent and models

`GET /api/agent` → `200`
```ts
{ id: string; name: string; description: string; version: string; placeholder: boolean;
  output_paths: string[];               // where the agent writes, from the bundle manifest
  inputs: AgentInput[] }                // only inputs the existing workflow genuinely needs (often empty)
type AgentInput = { name: string; label: string; help: string | null; required: boolean;
  kind: "text" | "select" | "path"; choices: string[] | null; default: string | null; max_length: number };
```

`GET /api/models` → `200`
```ts
{ items: { id: string; name: string; vendor: string; description: string | null;
           multiplier: number | null; is_default: boolean }[];
  default_model: string;
  locked_model: string | null;          // set when the agent file pins `model:` — selection is fixed
  lock_reason: string | null;
  entitlement_note: string }            // e.g. "Availability is enforced by GitHub Copilot for your account at run time."
```

### Jobs

`POST /api/bounded-context/jobs`
```ts
// request
{ owner: string; repo: string; base_branch: string; model: string; inputs?: Record<string, string> }
```
→ `202 Job`. Validates fields, checks the user can read the repo (and warns via `can_publish` if they cannot
push), checks the model against the catalog and agent lock, rejects active duplicates (`409 duplicate_job`).

`GET /api/bounded-context/jobs?status=<JobStatus>&owner=&repo=&page=1&per_page=20` → `200 Page<JobSummary>`
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

`POST /api/bounded-context/jobs/{job_id}/pull-request` (empty body) → `202 Job` with `publish.status = "queued"`.
Idempotent: if already queued/running/succeeded returns `200 Job` unchanged. `409 conflict` if the job has not
succeeded. `403 repository_access_denied` if the user lacks push permission.

`POST /api/bounded-context/jobs/{job_id}/cancel` → `202 Job` (`cancel_requested = true`, or `cancelled` if it
was still queued). `409 conflict` if already terminal.

`POST /api/bounded-context/jobs/{job_id}/retry` → `202 Job` — a **new** job with the same inputs (allowed for
`failed`/`cancelled`).

### Operations (no auth, not routed through the public ingress except `/healthz`)

| Path | Port | Purpose |
|---|---|---|
| `GET /healthz` | 8080 (api), 8081 (worker) | Liveness: process is up. |
| `GET /readyz` | 8080 (api), 8081 (worker) | Readiness: DB reachable, migrations applied; worker also checks the Copilot CLI and agent bundle. |
| `GET /metrics` | 9090 (api), 9091 (worker) | Prometheus metrics. |

A machine-readable version lives in `docs/openapi.yaml`.
