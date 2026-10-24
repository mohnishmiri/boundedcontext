# Operations runbook

## 1. Health and readiness

| Endpoint | Component | Meaning |
|---|---|---|
| `GET /healthz` (8080 / 8081) | api / worker | Process is up (liveness). |
| `GET /readyz` (8080) | api | Database reachable and all migrations applied. |
| `GET /readyz` (8081) | worker (loop mode) | Database ready **and** Copilot CLI answers `--version`. A worker that is not ready does not claim tasks. |
| `GET /metrics` (9090 / 9091) | api / worker | Prometheus metrics. Worker metrics are exported in `loop` mode only. |

## 2. Metrics

| Metric | Type | Labels | Use |
|---|---|---|---|
| `bcp_http_requests_total` / `bcp_http_request_seconds` | counter / histogram | route, method, status | API traffic, latency, 5xx rate |
| `bcp_api_errors_total` | counter | code | Error mix by stable error code |
| `bcp_auth_failures_total` | counter | code | Sign-in failures (`state_mismatch`, `exchange_failed`, …) |
| `bcp_api_rate_limited_total` | counter | | Clients hitting the per-minute limit |
| `bcp_jobs_created_total` | counter | | Demand |
| `bcp_sse_streams` | gauge | | Open progress streams |
| `bcp_github_requests_total` / `bcp_github_request_seconds` | counter / histogram | endpoint, status | GitHub health, rate limiting (`429`) |
| `bcp_github_token_refresh_total` | counter | | User token refreshes |
| `bcp_tasks_pending` | gauge | | Queue depth (also what KEDA scales on) |
| `bcp_tasks_running` | gauge | | In-flight tasks per worker |
| `bcp_tasks_claimed_total` | counter | kind | Throughput |
| `bcp_tasks_reaped_total` | counter | requeued | Workers that died mid-task |
| `bcp_generations_total` | counter | outcome, code | Success/failure mix (`agent_failed`, `copilot_unauthorized`, …) |
| `bcp_generation_seconds` / `bcp_agent_run_seconds` | histogram | | End-to-end and agent duration |
| `bcp_publish_total` | counter | outcome, code | Pull request success/failure |

Alert rules live in `deploy/observability/prometheus-rules.yaml`. Traces are exported over OTLP when
`config.otel.endpoint` is set. Logs are JSON on stdout with a `request_id` on every API request (also returned in
the `X-Request-Id` header and in every error body) — ask users for it when they report a problem.

## 3. Scaling

- **api / frontend**: HPA on CPU. Stateless; sessions live in PostgreSQL.
- **worker, loop mode**: KEDA ScaledObject on `pending + running` tasks, target `worker.concurrency` per replica.
  Scale-in can interrupt a run; the worker drains for `BCP_SHUTDOWN_GRACE_SECS`, then releases the task without
  consuming an attempt and the job restarts elsewhere. Keep `terminationGracePeriodSeconds` above the grace.
- **worker, once mode** (production default): KEDA ScaledJob creates one pod per pending task — strongest
  isolation, never interrupted by scale-in, 10–30 s start latency. `maxReplicaCount` bounds parallel agent runs
  (and Copilot spend).
- Database: connection count ≈ api replicas × `BCP_DATABASE_MAX_CONNECTIONS` + workers × (concurrency + 3).

## 4. Routine tasks

### Deploy / rollback

```bash
helm upgrade --install bcp deploy/helm/bounded-context-portal -n bcp -f values-prod.yaml --set ...image.tag=<v> --wait
helm -n bcp history bcp
helm -n bcp rollback bcp <REVISION> --wait
```

Migrations are forward-only and additive; rolling back the application does not roll back the schema. Write new
migrations so the previous release keeps working (expand → migrate → contract across releases).

### Rotate secrets

1. Add a new version of the secret in Key Vault.
2. Wait for the CSI rotation poll (`--rotation-poll-interval`, 5 min in the setup guide).
3. `kubectl -n bcp rollout restart deploy/bcp-api deploy/bcp-worker` (once mode: new Jobs pick it up automatically).

Notes: rotating the **GitHub App private key** — create the new key in GitHub first, roll out, then delete the old
key. Rotating the **client secret** — same order. Rotating **token-encryption-key** makes stored GitHub tokens
undecryptable: users are asked to sign in again; nothing else is lost.

### Upgrade Copilot CLI

1. Bump `COPILOT_CLI_VERSION` in `backend/Dockerfile`; build the worker image.
2. In staging run the smoke test (`docs/azure-setup.md` §7), including the `copilot --help` flag check.
3. Promote. `COPILOT_AUTO_UPDATE=false` guarantees the version does not drift at run time.

### Update the model catalog

Edit `config/models.yaml` (or `models.content` in Helm values) and redeploy. Only add identifiers confirmed with
`/model` in an interactive Copilot CLI session; keep new ones `verified: false` until then.

### Install or update the agent

See `agent-bundle/README.md`. The bundle version (content hash) is recorded on every job, so you can always tell
which agent produced a given document.

### Data retention

The API deletes settled jobs older than `BCP_JOB_RETENTION_DAYS` (default 90) every hour — generated files,
diffs and logs go with them. Audit events are kept. Back up PostgreSQL with the Flexible Server's automated
backups (point-in-time restore).

## 5. Troubleshooting by error code

| Code | Typical cause | Action |
|---|---|---|
| `github_auth_expired` | User token revoked/expired and refresh failed | User signs in again. Many at once → check the App's client secret. |
| `app_not_installed` | App not installed on that account/repository | Install the App (link in the UI). |
| `repository_access_denied` | User lacks access (or push, for publishing) | Grant access in GitHub. |
| `github_rate_limited` / `github_unavailable` | GitHub limits or outage | Retried automatically; check `bcp_github_requests_total{status="429"}`. |
| `copilot_unauthorized` | No Copilot seat, CLI disabled by policy, or App lacks a Copilot permission | See smoke test §7 in `docs/azure-setup.md`. |
| `model_unavailable` | Model not enabled for the user/org, or not in the catalog | Choose another model; fix `config/models.yaml`. |
| `model_locked_by_agent` | The agent file pins a model | Expected; the UI shows the lock. |
| `rate_limited` (job) | Copilot quota / premium requests exhausted, or the per-hour job limit | Wait; consider another model. |
| `agent_timeout` | Run exceeded `runtime.timeout_secs` / `BCP_MAX_JOB_TIMEOUT_SECS` | Large monorepo: use the scope input or raise the limit. |
| `agent_failed` | Non-zero exit of the CLI | Read the job log (redacted CLI output). |
| `agent_unavailable` | CLI missing or cannot reach Copilot | Check worker readiness and egress (§4 of the setup guide). |
| `output_missing` / `output_empty` / `output_invalid` / `output_too_large` | The agent did not write the expected file, or it was unusable | Compare with VS Code behaviour; check `output.paths` in `bundle.yaml`. |
| `repository_too_large` | Snapshot over `BCP_MAX_REPO_BYTES` / `BCP_MAX_REPO_FILES` | Raise the limits (and the worker's ephemeral storage). |
| `base_branch_changed` | The output file changed on the base branch after generation | User regenerates. |
| `branch_conflict` | A branch with the portal's name exists with other commits | Delete the branch and publish again; the portal never force-pushes. |

### Stuck jobs

- A job stays `queued`: check `bcp_tasks_pending`, worker readiness (`kubectl get pods`), KEDA (`kubectl get
  scaledjob,scaledobject -n bcp`, `kubectl describe` for trigger errors — usually the database secret for KEDA).
- A job stays `running` after a worker crash: the reaper returns it to the queue once the lease expires
  (`BCP_TASK_LEASE_SECS`, default 60 s), or fails it when attempts are exhausted.

### Useful SQL (read-only)

```sql
-- queue overview
SELECT kind, state, count(*) FROM job_tasks GROUP BY 1, 2 ORDER BY 1, 2;
-- recent failures by code
SELECT error_code, count(*) FROM jobs WHERE finished_at > now() - interval '1 day' AND status = 'failed' GROUP BY 1;
-- audit trail of a job
SELECT ts, actor_login, action, details FROM audit_events WHERE job_id = '<uuid>' ORDER BY id;
```
