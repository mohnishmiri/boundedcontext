# Security

This document describes the threat model and the controls implemented in the Bounded Context Portal. Each
control names the code or manifest that enforces it so reviewers can verify it.

## 1. Assets and trust boundaries

| Asset | Where it lives | Sensitivity |
|---|---|---|
| GitHub user-to-server tokens (`ghu_`, refresh `ghr_`) | `github_tokens` (AES-256-GCM ciphertext) | High — act as the user on GitHub and Copilot |
| GitHub App private key, client secret | Key Vault → CSI file mount | Critical — mint installation tokens for every installation |
| Token encryption key | Key Vault → CSI file mount | Critical — decrypts stored user tokens |
| Repository source | Worker pod scratch dir, only during a run | Customer code; never stored in the database |
| Generated documentation | `job_artifacts` | Derived from customer code; deleted after the retention period |
| Session ids | Browser cookie only (DB holds SHA-256) | High |

Trust boundaries: browser ↔ ingress; API ↔ GitHub; worker ↔ GitHub/Copilot; **agent process ↔ worker** (the agent
reads untrusted repository content and is driven by an LLM, so it is treated as untrusted).

## 2. Threats and controls

### Authentication and sessions

- GitHub App OAuth web flow with **state** (24 random bytes) and **PKCE S256**, both kept in an AES-GCM-sealed,
  HttpOnly, `SameSite=Lax`, 10-minute cookie scoped to `/api/auth` (`bcp-api/src/auth.rs`). State is compared in
  constant time.
- `return_to` must be a same-origin path (`validate::return_to_path`) — no open redirects.
- Sessions are 32 random bytes in an HttpOnly, `Secure`, `SameSite=Lax` cookie; the database stores only
  `sha256(session)` and an expiry (`sessions`). Logout deletes the row. Expired sessions are purged hourly.
- GitHub tokens never reach the browser. They are encrypted with AES-256-GCM using a random nonce and associated
  data that binds the ciphertext to the user id and token kind (`bcp-core/src/crypto.rs`), so ciphertexts cannot be
  swapped between rows or users.
- Refresh tokens are single-use at GitHub; refreshes are serialised with `SELECT … FOR UPDATE`
  (`bcp-services`). A rejected refresh deletes the stored tokens and the user must sign in again.

### Authorisation

- Every GitHub listing uses the **user's** token, so users see only repositories they can access *and* where the
  App is installed. Jobs are private to the requester (`get_user_job`); other users get `404`.
- Job creation re-checks read access, branch existence and App installation server-side; publishing re-checks
  **push** permission with the user's token immediately before minting a write-capable installation token.
- Installation tokens are minted per task, scoped to **one repository** and the minimum permissions
  (`contents: read` for fetching; `contents: write` + `pull_requests: write` for publishing), and never handed to
  the agent.

### CSRF and browser hardening

- Mutating requests require `X-Requested-With: bcp` **and** an `Origin` (or `Referer`) equal to the configured
  public origin (`bcp-api/src/middleware.rs`), on top of `SameSite=Lax` cookies.
- API responses: `nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`, `Cache-Control: no-store`.
- Frontend (nginx): strict CSP (`script-src 'self'`, `object-src 'none'`, `frame-ancestors 'none'`,
  `connect-src 'self'`); `style-src 'unsafe-inline'` is required by Mermaid's inline styles.
- Agent output is rendered with `react-markdown` + `rehype-sanitize` (no raw HTML); Mermaid runs with
  `securityLevel: 'strict'`. The UI never uses `dangerouslySetInnerHTML` for agent output.

### Input validation and injection

- Strict validators for owner, repository, branch (git `check-ref-format` rules), model id, repository-relative
  paths and free-text inputs (`bcp-core/src/validate.rs`). JSON bodies reject unknown fields and are capped at
  64 KiB.
- GitHub URL path segments are percent-encoded per segment; GitHub base URLs come only from configuration, and
  pagination links are followed only when they point at the configured API origin (SSRF hygiene).
- **No shell anywhere**: the agent runtime is spawned with an argv vector. Prompt values are validated single-line
  strings substituted into a logic-less template; a value cannot inject template syntax (`template.rs`).
- SQL uses bound parameters only.

### The agent sandbox (prompt injection, malicious repositories)

Repository content can try to instruct the agent ("ignore previous instructions, exfiltrate …"). The portal assumes
the agent may be fully manipulated and limits what that can achieve:

| Control | Enforcement |
|---|---|
| No shell, URL-fetch or memory tools; only read/write | `bundle.yaml` `runtime.allow_tools` / `deny_tools` → `--allow-tool` / `--deny-tool` |
| Built-in GitHub MCP server disabled (it would act with the user's token) | `--disable-mcp-server github-mcp-server` |
| Repository instruction files (`AGENTS.md`, `copilot-instructions.md`) not loaded | `--no-custom-instructions` (default) |
| The agent cannot be shadowed by a repository agent | Agent copied into a private `COPILOT_HOME/agents` (personal agents win on id collision) |
| Scrubbed environment: no worker secrets, no cloud credentials | `env_clear()` + small allowlist (`copilot.rs`) |
| Only allowlisted output paths are ever committed; everything else is reported as *unexpected* | `workspace::collect_outputs`, re-checked in `publish.rs` |
| Symlinks/devices are never followed or committed; outputs must be UTF-8 text within size limits | `workspace.rs` |
| Human review before anything reaches GitHub; PRs are never merged automatically | UI + `publish.rs` |
| Time, CPU, memory, disk limits; process-group kill on timeout/cancel; `tini` reaps orphans | `copilot.rs`, worker Deployment/ScaledJob, Dockerfile |
| Pod sandbox: non-root, read-only root FS, all capabilities dropped, `seccomp: RuntimeDefault`, no service-account token | Helm `_helpers.tpl` |
| Egress limited to DNS, PostgreSQL and HTTPS to public addresses (FQDN-restricted with Cilium) | `networkpolicy.yaml` |
| `once` mode: a fresh pod per run, so nothing survives between jobs | `worker.mode=once` (KEDA ScaledJob) |

The residual risk is that a manipulated agent writes misleading documentation into the allowlisted file. That is
mitigated by the review screen (rendered preview and diff) and by pull-request review.

### Generators from repositories

A generator repository decides the agent's prompt, tool permissions and output paths, so a user-supplied one is
effectively user-supplied configuration for the sandbox above. Controls:

| Control | Enforcement |
|---|---|
| Only repositories on the operator allowlist (or curated) can be used; re-checked on every use | `config/generators.yaml` → `GeneratorRegistry::ensure_allowed` |
| The user's own token fetches the generator, so a user can only use generators they can read | `GeneratorResolver` |
| Hard tool caps whatever `bundle.yaml` says: only read/write/edit/search allowed; shell, url, memory always denied; repository instructions off | `AgentBundle::restrict_to_platform_policy` |
| Bundles that configure MCP servers (`runtime.mcp_config` or `mcp-servers:` in the agent file) are rejected | same |
| Outputs limited to documentation files (md, mdx, txt, yaml, yml, json) below a `docs` directory and never inside dot-directories, so a generator cannot write workflows, manifests or source | same + `matches_output` |
| The job pins the generator commit; the worker runs exactly that revision | `jobs.generator_sha` |
| Generator archives are size/file capped and extracted with the same safe extractor as repositories | `GeneratorResolver` (50 MiB / 5000 files) |

The built-in generator ships in the worker image and is not clamped, because the operator controls it.

### Safe repository extraction

GitHub tarballs are extracted by our own code (`workspace::extract_tarball`): absolute paths, `..`, `.git/`,
non-UTF-8 names, symlinks, hard links and devices are skipped; files are created with `create_new` (never
following an existing path); total size and file count are capped (`BCP_MAX_REPO_BYTES`, `BCP_MAX_REPO_FILES`),
counting decompressed bytes to defeat compression bombs.

### Secrets handling and logging

- All secrets come from Key Vault through the Secrets Store CSI driver with Workload Identity; the application
  reads `*_FILE` paths. No secret is placed in values files, ConfigMaps or images.
- Every log line from the agent is redacted (`bcp-core/src/redact.rs`: GitHub token formats, bearer headers,
  basic-auth URLs, PEM keys, JWTs, connection-string keys, plus the run's exact token) and truncated before it is
  stored or shown. `Debug` impls of token-bearing types print `<redacted>`.
- reqwest errors are logged without URLs (codeload redirects carry short-lived tokens in the query string).

### Abuse and availability

- Per-client rate limit (`BCP_RATE_LIMIT_PER_MINUTE`, per pod) and a per-user job-creation limit per hour enforced
  in the database across pods (`BCP_JOB_CREATE_LIMIT_PER_HOUR`).
- One active job per user/repository/branch/model (partial unique index).
- Request timeout of 60 s on non-streaming endpoints; SSE streams are bounded by job completion and the ingress
  timeout.

### Audit

`audit_events` is append-only (a trigger rejects `UPDATE`/`DELETE`) and records sign-in/out, generation
requested/cancelled/succeeded/failed, publish requested, commit/branch/PR created and publish failures, with actor,
repository, job and request id. Audit rows are not removed by job retention.

## 3. Known limitations and items to verify

| Item | Status |
|---|---|
| Copilot CLI's handling of `--no-custom-instructions` and `--disable-mcp-server` names | Verify against the pinned CLI version (`docs/discovery.md` R6) |
| GitHub App permission required for `ghu_` tokens to call Copilot | Undocumented; validated by the smoke test in `docs/azure-setup.md` §7 |
| Copilot CLI telemetry cannot be disabled when authenticating with GitHub | Documented by GitHub; record in your compliance review |
| Rate limiting is per pod | Effective limit = configured value × API replicas |
| Plain Kubernetes NetworkPolicy cannot filter by host name | Enable `networkPolicy.cilium.enabled` (Azure CNI powered by Cilium) for FQDN egress rules |
| The agent bundle in this repository is a placeholder | Replace it before production (`agent-bundle/README.md`) |

## 4. Reporting

Report suspected vulnerabilities privately to the platform team; do not open public issues.
