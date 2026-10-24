# Phase 1 — Discovery findings

Date: 2026-10-24. This records what was inspected before any implementation decision, what was found,
and what could not be verified. Anything in the "Unverified" column must be confirmed before production use.

## 1. The existing bounded-context agent

| Item | Finding |
|---|---|
| Agent definition in this repository | **Not present.** `bounded-context/` was empty when work started. |
| Agent definition elsewhere on the workstation | **Not found.** Searched `~/GITHUB/**` (text search for "bounded context"/"bounded-context"/"boundedcontext"), all `*.agent.md`, `*.prompt.md`, `*.chatmode.md`, `copilot-instructions.md`, `AGENTS.md`, VS Code user prompts (`~/Library/Application Support/Code/User/prompts` — empty) and the Copilot CLI synced-customisation plugins (`~/Library/Application Support/Code/agentPlugins/...`). |
| Owner decision | The repository owner chose to supply the agent files later and asked to proceed. Runtime chosen: **GitHub Copilot CLI**. |

Consequence: the platform is built around an **agent bundle** (see `agent-bundle/README.md`) that holds the
agent files *verbatim* plus a small manifest that records the facts the spec asked us to discover (prompt,
output path, tools). The bundle shipped in this repository is a **clearly labelled placeholder** so the
end-to-end flow can be exercised. Replacing it with the real agent is a file copy plus manifest edit — no code
change — unless the real agent depends on capabilities listed in §4 that do not exist outside VS Code.

**Values that are placeholders until the real agent arrives** (all live in `agent-bundle/bundle.yaml`):
`agent.id`, `prompt.template`, `output.paths`, `output.primary`, and the tool allow/deny lists.

## 2. Conventions observed in the owner's other agents (`~/GITHUB/ops-portal`)

These are the closest evidence of how the real agent is likely written:

- Agents live in `.github/agents/<Name>.agent.md` and are mirrored byte-for-byte into `.claude/agents/`.
- Frontmatter: `name`, `description`, `tools: [...]`, `agents: [...]`.
- `tools` uses **VS Code tool IDs** (`execute/runInTerminal`, `edit/createFile`, `search/codebase`,
  `read/readFile`, `agent/runSubagent`, `todo`) and **MCP tool IDs** from `azure-mcp/*`, `com.microsoft/azure/*`
  and `levelup/*` servers.
- Prompt files live in `.github/prompts/*.prompt.md` with `description`, `argument-hint`, `agent` frontmatter
  and `{{placeholder}}` variables.
- VS Code also syncs agents/prompts into a Copilot CLI plugin layout
  (`agents/*.agent.md`, `commands/*.prompt.md`, `.mcp.json`, `.plugin/plugin.json`), confirming these files are
  consumable by Copilot CLI.

## 3. Copilot CLI facts (from docs.github.com, fetched 2026-10-24; npm `@github/copilot` latest = 1.0.103)

| Topic | Documented behaviour | Used how |
|---|---|---|
| Non-interactive run | `copilot -p PROMPT`, `-s` (response only), `--no-ask-user` | Adapter argv |
| Custom agent | `--agent=ID`; agents discovered in `.github/agents/` (repo) and `~/.copilot/agents/` (user). **Personal agent wins on ID collision.** ID = file name without `.agent.md`. | Bundle agent is copied into a per-job `$COPILOT_HOME/agents/`, so a target repository cannot shadow it. |
| Model | `--model=MODEL`, `COPILOT_MODEL`; **a `model:` in the agent frontmatter overrides `--model`**. `auto` lets Copilot choose. | Adapter detects a pinned frontmatter model and the API refuses a conflicting selection instead of silently ignoring it. |
| Listing models | **No non-interactive command or API is documented** (only interactive `/model`). The Copilot API's `GET /models` is what editors use and is reported to return the calling account's models, but it is undocumented and unverified with GitHub App user tokens. | The API tries `GET {BCP_COPILOT_API_URL}/models` with the user's token (best-effort, cached) and falls back to the operator catalog (`config/models.yaml`) with a visible warning. Verify with the smoke test in `docs/azure-setup.md` §7; `Copilot-Integration-Id` is configurable (`BCP_COPILOT_INTEGRATION_ID`). |
| Tools / permissions | `--allow-tool`, `--deny-tool` (kinds `shell`, `write`, `read`, `url`, `memory`, MCP server name), `--available-tools`, `--excluded-tools`. Unrecognised agent `tools:` names "are ignored". | Least-privilege defaults: no shell, no URL fetch, no MCP. |
| MCP | `--disable-mcp-server=NAME`, `--additional-mcp-config` (from command-reference summary). | Built-in GitHub MCP server disabled by default. |
| Auth | Precedence `COPILOT_GITHUB_TOKEN` > `GH_TOKEN` > `GITHUB_TOKEN` > keychain. Accepted: `gho_`, `github_pat_` (with *Copilot Requests*), **`ghu_` GitHub App user-to-server tokens via env var**. `ghp_` rejected. | The requesting user's GitHub App user token is passed as `COPILOT_GITHUB_TOKEN`, so each run uses **that user's** Copilot entitlement. |
| State | `COPILOT_HOME` replaces `~/.copilot`; `COPILOT_AUTO_UPDATE=false` pins version; `--log-dir`; `--share=PATH` exports transcript. | Per-job `COPILOT_HOME`, pinned CLI version in the worker image. |
| Output | `--output-format json` emits JSONL; **schema for `-p` not documented**. | Parsed leniently for stage hints; raw lines kept as redacted logs. |

## 4. Risks and unverified items

| # | Item | Status | Mitigation in this implementation |
|---|---|---|---|
| R1 | Real agent not available | **Blocker for acceptance criterion 3/4** | Bundle abstraction + placeholder; swap documented in `agent-bundle/README.md`. |
| R2 | Real agent may rely on VS Code-only tools (`execute/runInTerminal`, `search/codebase`, `vscode`) or MCP servers (`levelup`, `azure-mcp`) | Unknown until agent arrives | Copilot CLI ignores unknown tool names; CLI built-ins provide read/edit/search. MCP servers needing corporate auth must be explicitly added to the bundle (`mcp_config`) — off by default. |
| R3 | Required GitHub App permission for `ghu_` tokens to use Copilot (e.g. an account permission such as *Copilot Requests*) | **Not documented** | Worker preflight reports a distinct `copilot_unauthorized` error. Validate with a smoke test (`docs/azure-setup.md` §7). |
| R4 | Exit codes of `copilot -p` | **Not documented** | Success requires exit 0 **and** a valid output file; stderr classified best-effort. |
| R5 | Whether unknown models fail fast | Not documented | Catalog validation before enqueue; runtime failures mapped to `model_unavailable` when detectable, never silently substituted. |
| R6 | Repository instructions (`copilot-instructions.md`, `AGENTS.md`) in the *target* repo are loaded automatically and are untrusted | By design of Copilot | Blast-radius limits (no shell/URL/MCP, path allowlist on outputs, human review, no auto-merge). Manifest flag to pass `--no-custom-instructions` (flag name from a docs summary — verify). |
| R7 | Copilot CLI telemetry cannot be disabled in GitHub-auth mode | Documented | Recorded for compliance review. |
| R8 | Copilot entitlement per user is not queryable ahead of time with a user token | No documented API | Surfaced as a runtime error with clear wording. |

## 5. What was not invented

- Output file name/location: taken from the bundle manifest only. The placeholder uses
  `docs/bounded-context.md`, marked as a placeholder.
- The agent prompt: placeholder text in the manifest, clearly marked.
- Model list: from an operator-edited catalog seeded from the "Supported AI models in GitHub Copilot" page;
  identifiers must be confirmed with `/model` in an interactive Copilot CLI session for your plan.
