---
description: >
  Standalone orchestrator that generates either a FULL 16-artifact pack or a LEAN merged
  pack for ONE service or codebase (any language/stack) per run, plus one bounded-context
  document. Requires MS_ROOT_PATH; accepts ARCHETYPE_PROFILE=FULL|LEAN. Stops immediately
  if MS_ROOT_PATH is missing. SERVICE_NAME is derived automatically from the folder name.
  Use when asked to "generate the archetype pack", "run the archetype orchestrator", or
  "document <service>".
---

# AIFC Archetype Orchestrator — Profile-driven pack + bounded context, ONE service per run

You are the **orchestrator**. You do NOT write archetype content yourself. Your job is to:
1. Validate the required input — stop hard if missing.
2. Resolve archetype profile (`FULL` or `LEAN`) and expected output set.
3. Run a deterministic ingress inventory before any batch.
4. Fan out 5 sequential subagent batches, each grounded in a shared evidence file.
5. Run a dedicated bounded-context subagent batch using `prompts/universal-bounded-context.prompt.md`.
6. Gate on flow parity and Mermaid diagram validity before declaring GO.

---

## ⛔ HARD STOP — Required Input

Before doing ANYTHING else, confirm the input is provided by the user:

| Variable | Required | Description |
|----------|----------|-------------|
| `MS_ROOT_PATH` | ✅ YES | Absolute path to the target service/codebase root folder |
| `ARCHETYPE_PROFILE` | Optional (default `LEAN`) | Output profile. Allowed values: `FULL`, `LEAN` |

**If `MS_ROOT_PATH` is not provided:** stop, print exactly:
```
⛔ Missing required input: MS_ROOT_PATH
Usage: provide MS_ROOT_PATH = <absolute path to the service/codebase root>
Example: MS_ROOT_PATH = H:\projects\...\apm0014313-attccc-calcengine
```

**Do not attempt to guess, infer, or auto-discover this value.**

If `ARCHETYPE_PROFILE` is omitted, set it to `LEAN`.
If provided and not one of `FULL` or `LEAN`, stop and print exactly:
```
⛔ Invalid ARCHETYPE_PROFILE
Allowed values: FULL | LEAN
Example: ARCHETYPE_PROFILE = FULL
```

---

## Derived Paths (set after input is confirmed)

```
MS_ROOT_PATH  = <as provided>
ARCHETYPE_PROFILE = <FULL | LEAN; default LEAN>
SERVICE_NAME  = <basename of MS_ROOT_PATH, e.g. the final path segment>
DOCS_DIR      = ${MS_ROOT_PATH}/docs/
OUTPUT_DIR    = ${MS_ROOT_PATH}/docs/archetypes/
EVIDENCE_FILE = ${MS_ROOT_PATH}/docs/archetypes/_archetype-evidence.md
BCTX_PROMPT   = prompts/universal-bounded-context.prompt.md
BCTX_FILE     = ${MS_ROOT_PATH}/docs/BOUNDED-CONTEXT-${SERVICE_NAME}.md
```

`BCTX_FILE` must be written at `docs/` level (sibling of `docs/archetypes/`), never inside
`docs/archetypes/`.
`BCTX_PROMPT` is resolved from this generator workspace, not from the target microservice.

---

## Output Profiles

Generated under `OUTPUT_DIR`. File names, numbering, and required sections are fixed.
Profile values are control flags only and must never be prefixed to artifact file names.

### Profile `FULL` (16 archetypes)

| # | File | Purpose |
|---|------|---------|
| 00 | `00-IDENTITY-RECONCILIATION.md` | Code-grounded service definition; precondition for all other artifacts |
| 01 | `01-README.md` | Operator entry point; pack overview and index |
| 02 | `02-ARCHITECTURE.md` | Current-state architecture record |
| 03 | `03-SERVICE-MANIFEST.yaml` | Machine-readable canonical manifest |
| 04 | `04-ADRs/` | Architecture Decision Records (one file per decision + README index) |
| 05 | `05-component-diagram.md` | Mermaid component diagram |
| 06 | `06-sequence-diagrams.md` | Mermaid sequence diagrams — one per ingress path |
| 07 | `07-data-model.md` | Persistence / data model |
| 08 | `08-error-taxonomy.md` | Failure-mode catalog |
| 09 | `09-tier3-service-rules.md` | Reconciled Tier-3 service rules |
| 10 | `10-service-memories-seed.md` | Curated context bundle for AI agents / new engineers |
| 11 | `11-test-strategy.md` | Test strategy + coverage gaps |
| 12 | `12-threat-model.md` | Threat model (control/trust-boundary view) |
| 13 | `13-observability-contract.md` | Metrics, health endpoints, named loggers |
| 14 | `14-runbook.md` | On-call playbook: triage flow + startup checklist |
| 15 | `15-tribal-knowledge.md` | Undocumented senior-engineer warnings |

### Profile `LEAN` (4 archetypes, merged where safe)

| # | File | Purpose |
|---|------|---------|
| 00 | `00-IDENTITY-RECONCILIATION.md` | Code-grounded service definition; precondition for all other artifacts |
| 01 | `01-SERVICE-FOUNDATION.md` | Merged README + architecture + ADR decisions + component/sequence flows + data model |
| 02 | `02-SERVICE-MANIFEST.yaml` | Machine-readable canonical manifest |
| 03 | `03-ASSURANCE-AND-OPERATIONS.md` | Merged risk/governance + quality/security/observability + runbook + tribal knowledge |

Also write `00-INDEX.md` and `ARCHETYPE-SUMMARY.md` in the final batch.
Also write `BOUNDED-CONTEXT-${SERVICE_NAME}.md` at `DOCS_DIR` using
`prompts/universal-bounded-context.prompt.md`.

### Per-file required content (profile rules)
- **00** — code-derived one-line service definition; "authoritative source = the code" statement.
- **01** — what service does / does NOT do; dependencies; build/test commands; ports; ownership.
- **02** — system context; components; runtime topology; all ingress paths; constraints.
- **03** — YAML per schema in `schema/service-manifest.schema.yaml`. Every value cites source.
- **FULL 04** — one ADR per material decision; README index; each ADR: Context / Decision / Consequences.
- **FULL 05** — Mermaid flowchart of internal blocks (security, endpoints, BL, processors, DAOs) and wiring.
- **FULL 06** — one Mermaid sequenceDiagram per distinct ingress path from the Ingress Inventory.
  Do not consolidate unrelated flows. Every active path in the inventory = one diagram.
- **FULL 07** — tables/schemas owned or touched; Oracle/Mongo/Redis/Blob mappings.
- **FULL 08** — failure modes, swallowed exceptions, dead-letter weaknesses.
- **FULL 09** — service-specific guardrails reconciled against platform rules.
- **FULL 10** — curated context bundle; key design decisions; common pitfalls; mental model.
- **FULL 11** — test coverage numbers; quality-gate posture; gaps flagged as risks.
- **FULL 12** — Veracode/SCA ratings; logging exposure; EOL dependencies; control/trust-boundary view.
- **FULL 13** — metrics, health endpoints, named loggers and their meaning.
- **FULL 14** — symptoms -> diagnostics -> recovery steps; triage flow diagram; startup checklist.
- **FULL 15** — "learned by getting burned" warnings not in any other doc.
- **LEAN 01** — merged content from 01-README, 02-ARCHITECTURE, 04-ADRs, 05-component-diagram, 06-sequence-diagrams, 07-data-model in one file with clear section headers.
  Sequence-diagram rule is unchanged: one active ingress path = one diagram.
- **LEAN 02** — YAML manifest equivalent to FULL 03-SERVICE-MANIFEST.yaml.
- **LEAN 03** — merged content from 08-error-taxonomy, 09-tier3-service-rules, 10-service-memories-seed,
  11-test-strategy, 12-threat-model, 13-observability-contract, 14-runbook, 15-tribal-knowledge.

---

## Batching

6 sequential batches total.
- Batches 1-5: archetype generation via `prompts/batch-subagent.prompt.md`.
- Batch 6: bounded-context generation via `prompts/universal-bounded-context.prompt.md`.

For archetype batches, the evidence file is the only state carrier between batches — batch N+1
reads it before doing any analysis.

| Batch | FULL artifacts | LEAN artifacts | Reads from evidence |
|-------|-----------|-------------------|
| 1 — Identity & Foundation | 00, 01, 02, 03 | 00, 02 | — (seeds evidence) |
| 2 — Design & Flow | 04-ADRs/, 05, 06, 07 | 01 | Batch 1 (identity, tech stack, manifest) |
| 3 — Failure & Governance | 08, 09, 10 | 03 (risk/governance sections) | Batches 1–2 |
| 4 — Quality, Security & Operations | 11, 12, 13 | 03 (quality/security/observability sections) | Batches 1–3 |
| 5 — Close-out | 14, 15, 00-INDEX.md, ARCHETYPE-SUMMARY.md | 03 (runbook/tribal sections), 00-INDEX.md, ARCHETYPE-SUMMARY.md | Batches 1–4 |

| Batch | Bounded context output | Prompt | Notes |
|-------|-------------------------|--------|-------|
| 6 — Bounded Context | `docs/BOUNDED-CONTEXT-${SERVICE_NAME}.md` | `prompts/universal-bounded-context.prompt.md` | Must be outside `docs/archetypes/` |

---

## Invariants (never violate)

1. **Grounding.** Every fact traces to actual source/config/test read from `MS_ROOT_PATH`. No fabrication, no trusting doc claims — re-verify the sink.
2. **Single-service scope.** No batch may reference any other service/codebase.
3. **Append-only evidence.** Never delete or rewrite a prior batch section — later batches build on top.
4. **Sequential batches.** 1→6 in order; do not parallelize.
5. **Deterministic outputs.** Archetype file set and manifest schema are fixed by selected profile;
  bounded-context output path is fixed by this spec.

---

## Orchestrator Procedure

### Step 0 — Preflight
1. Confirm `MS_ROOT_PATH` is provided. If not → HARD STOP (print usage above).
2. Resolve `ARCHETYPE_PROFILE` to `FULL` or `LEAN` (default `LEAN`), and stop on invalid value.
3. Derive `SERVICE_NAME` from the final path segment (basename) of `MS_ROOT_PATH`.
4. Verify `MS_ROOT_PATH` exists on disk as a directory. If not → stop and report the invalid path.
  Do **not** check for any particular subfolder, file, or language convention at this stage —
  existence of `MS_ROOT_PATH` itself is the only precondition. Stack/framework detection and
  source-root discovery happen in Step 0.5 and may fail on their own terms if no recognizable
  source code is found.
5. Verify `BCTX_PROMPT` exists at `prompts/universal-bounded-context.prompt.md` in this generator workspace.
  If missing → stop and report invalid prompt path.
6. Create `OUTPUT_DIR` if it does not exist.
7. If `EVIDENCE_FILE` does not exist, create it from the template in the Appendix below,
   seeded with `SERVICE_NAME` and empty batch headings 1–5.

---

### Step 0.5 — Deterministic Ingress Inventory (MANDATORY, orchestrator-run, before Batch 1)

Do NOT delegate this to a subagent. The orchestrator runs these searches itself.
Prior runs missed ingress paths because discovery was left to ad-hoc subagent reading.
This step is stack-agnostic — it works the same way whether the codebase is Java, Python,
TypeScript/Node, .NET, Go, or something else; only the concrete patterns searched for change.

**0.5-A — Detect the stack and locate entry-point directories**

Scan `MS_ROOT_PATH` directly — no fixed subfolder (e.g. `service/src/main/java/`) is assumed.
Use `list_dir`/search to identify the dominant technology stack(s) present, based on build/config
files and file extensions found anywhere under `MS_ROOT_PATH`:
- **Java/Kotlin/Scala** — `pom.xml`, `build.gradle`, `*.java`/`*.kt`/`*.scala`
- **Python** — `requirements.txt`, `pyproject.toml`, `*.py`
- **TypeScript/JavaScript/Node** — `package.json`, `*.ts`/`*.js`
- **.NET/C#** — `*.csproj`/`*.sln`, `*.cs`
- **Go** — `go.mod`, `*.go`
- **SQL/DB-centric** — `*.sql`, migration folders

More than one stack may apply (e.g. a Java service with Python data jobs). Then locate the
directories that group entry points, wherever they actually live in the tree — common names
include `controller/`, `controllers/`, `routes/`, `endpoints/`, `handlers/`, `api/`, `resources/`,
`consumers/`, `listeners/`, `jobs/`, `functions/` — but do not assume any fixed path convention.
Each such directory represents an entry-point group and must produce at least one row in the
inventory table below. A directory with zero rows is a bug in the inventory — re-examine it
before continuing.

If no recognizable source files can be found anywhere under `MS_ROOT_PATH` → stop and report
that no source code was found; do not fabricate an inventory.

**0.5-B — Entry-point / route scan (stack-appropriate)**

For every source file under an entry-point directory, search for the ingress patterns that
match the detected stack(s):
- **Java/Kotlin/Scala** — class-level `@Path("...")` + method-level `@GET`/`@POST`/`@PUT`/
  `@DELETE`/`@PATCH` (JAX-RS), or `@RequestMapping`/`@GetMapping`/`@PostMapping`/etc. (Spring).
  Concatenate class path + method path to form the full route (e.g. `@Path("/cache")` class +
  `@Path("/unacceptTransRulesCache")` method → `/cache/unacceptTransRulesCache`).
- **Python** — Flask (`@app.route`, `@blueprint.route`), FastAPI (`@app.get`/`@app.post`/etc.),
  Django (`urls.py` path patterns), or PySpark/notebook job entry points.
- **TypeScript/JavaScript/Node** — Express/Fastify route registrations (`app.get(...)`,
  `router.post(...)`), NestJS decorators (`@Get()`, `@Post()`, `@Controller(...)`).
- **.NET/C#** — `[Route]`/`[HttpGet]`/`[HttpPost]`/etc. controller actions, or minimal-API
  registrations (`app.MapGet`, `app.MapPost`).
- **Go** — `net/http` handler registrations, or router-framework route registrations
  (gin/echo/fiber `.GET(...)`, `.POST(...)`, etc.).

Each matched method/handler = one inventory row. Also read 3–5 surrounding lines to determine
if the handler body is commented out — mark as inactive if so.

**0.5-C — Messaging/eventing/scheduled ingress scan (stack-appropriate)**

Under the full `MS_ROOT_PATH` tree, search for messaging and scheduled ingress appropriate to
the detected stack(s), for example:
- **Messaging/eventing** — Kafka/EventHub/ServiceBus/RabbitMQ/SQS consumer classes or modules;
  SDK client usages (`EventProcessorClient`, queue/topic receiver clients); framework decorators
  such as `@KafkaListener`, `@JmsListener`, `@RabbitListener`; Python `celery.task`/consumer
  loops; Node queue-processor registrations (e.g. BullMQ); serverless triggers (Azure
  Functions/AWS Lambda event bindings).
- **Scheduled/cron** — `@Scheduled`, Quartz trigger classes, cron-style trigger config, Celery
  beat schedules, `node-cron`/APScheduler jobs, Kubernetes `CronJob` manifests.

For each hit, read enough of the surrounding code/config to determine:
- The queue/topic/schedule name (from config/properties/env reference or literal argument).
- Whether the registration or its construction is commented out → inactive.

**0.5-D — Write the Ingress Inventory to EVIDENCE_FILE**

Append to `EVIDENCE_FILE` under `## Ingress Inventory (deterministic, orchestrator-run)`:

```markdown
| route / queue name | class/module.method | kind | active (Y/N) | source file |
|--------------------|---------------------|------|--------------|------------|
| /cache/unacceptTransRulesCache | CalcCacheServices.getUnacceptTransRulesCache | REST GET | Y | service/cache/CalcCacheServices.java |
| … | … | … | … | … |
```

(The row above is illustrative only — actual entries reflect whatever stack/framework
conventions were detected in 0.5-A/B/C for this codebase.)

This table is ground truth for Steps 1–2.5. Every artifact listing ingress reconciles against
it — not against a subagent's own re-derivation.

**0.5-E — Validate completeness**

Count inventory rows per entry-point directory identified in 0.5-A. If any such directory has
zero rows, stop — re-examine that directory before proceeding to Batch 1.

---

### Step 1 — Run Archetype Batches 1 through 5, in order

For each batch, call `runSubagent` with the **Per-Batch Subagent Prompt** from
`prompts/batch-subagent.prompt.md`, substituting:
- `${SERVICE_NAME}`, `${MS_ROOT_PATH}`, `${OUTPUT_DIR}`, `${EVIDENCE_FILE}`
- `${ARCHETYPE_PROFILE}`
- The batch number and profile-specific artifact list from the Batching table above
- For batches 1 and 2: inline the full Ingress Inventory table from Step 0.5 so the
  subagent starts from a complete checklist, not a rediscovery task.

Do NOT start batch N+1 until batch N has returned and you have confirmed (by reading
`EVIDENCE_FILE`) that it appended its section.

---

### Step 1.5 — Run Batch 6 (Bounded Context)

Call `runSubagent` with `prompts/universal-bounded-context.prompt.md`, providing:
- area/service/flow scope = `${SERVICE_NAME}`
- codebase root = `${MS_ROOT_PATH}`
- output target = `${BCTX_FILE}`
- instruction: save bounded context at docs level and not inside `${OUTPUT_DIR}`
- `EVIDENCE_FILE` path = `${EVIDENCE_FILE}`

Batch 6 runs only after archetype Batch 5 completes.

**After writing `${BCTX_FILE}`, the subagent MUST append to `${EVIDENCE_FILE}` under
`## Batch 6 — Bounded Context` (append-only):**

```markdown
### Key files read
- <list>

### Facts established
- <key design decisions, invariants, and boundaries confirmed by the code>

### Artifacts written
- ${BCTX_FILE}

### Open questions added to Gap Register
- <count and list>
```

Add one Gap Register row per unresolved item flagged in Section 9 of the bounded-context document.

---

### Step 2 — Reconcile

1. Read `EVIDENCE_FILE` end to end. Confirm all 6 batch sections are present, each with:
   (a) key files read, (b) facts established, (c) artifacts written, (d) gap rows.
2. Verify profile-expected outputs on disk:
  - `FULL`: all 16 artifacts + `00-INDEX.md` + `ARCHETYPE-SUMMARY.md`.
  - `LEAN`: 4 artifacts (00-03) + `00-INDEX.md` + `ARCHETYPE-SUMMARY.md`.
3. Verify bounded-context output exists at `${BCTX_FILE}` and does not exist under `${OUTPUT_DIR}`.

---

### Step 2.5 — Flow Parity & Diagram Validation (MANDATORY before Step 3)

Do not proceed to the final report until both checks pass.

**Check A — Flow parity (cross-reference all three artifacts against the Ingress Inventory)**

Build a parity table with every row from the Ingress Inventory. Use profile-specific columns:

- `FULL` columns:
  `inventory | in 05-component | in 06-sequence (one diagram?) | in 02-ARCHITECTURE ingress table | status`
- `LEAN` columns:
  `inventory | in 01-SERVICE-FOUNDATION component section | in 01-SERVICE-FOUNDATION sequence section (one diagram?) | in 01-SERVICE-FOUNDATION ingress table | status`

| route / queue | inventory | profile component column | profile sequence column (one diagram?) | in 02-ARCHITECTURE ingress table | status |
|---|---|---|---|---|---|

Rules:
- Every **active** inventory row must appear in all three columns → ✅.
- Every **inactive** inventory row must be explicitly noted as inactive in any column that
  mentions it (not silently omitted) → ✅ if correctly labeled, ❌ if presented as active.
- Any ❌ must be fixed directly in the artifact file before continuing — do not defer to
  the Gap Register.
- Consolidated/merged flows inside one sequence diagram (two unrelated ingress paths in one
  diagram) -> ❌ in both profiles.

**Check B — Mermaid rendering validation (all generated artifacts)**

Run this check against all generated artifacts for the selected profile.

Scan every Mermaid code block across the generated artifact files for:
1. Unquoted `participant`/node aliases containing `(`, `)`, `:`, `/`, or special characters
   → wrap in `"..."`.
2. `;` inside `Note over`, `Note right of`, or any label text → replace with `,`.
3. `<br/>` or HTML tags inside labels → replace with plain text.
4. Keyword casing errors (`note over` → `Note over`).
5. Unbalanced `alt`/`opt`/`loop`/`end` blocks.

Fix any issue found directly in the file. Do not log as a gap and move on.

---

### Step 2.6 — Bounded Context Validation (MANDATORY)

Validate `${BCTX_FILE}` before final report:
1. Contains all 9 required sections from `universal-bounded-context.prompt.md` in order.
2. Section 1 includes exactly one Mermaid `flowchart LR` context map.
3. Section 6 includes at least one Mermaid `flowchart TD` flow diagram.
4. Sections 7 and 9 are populated (not empty, not `N/A`).
5. Evidence tags are present for invariant/rule/data claims per prompt rules.

If any check fails, fix `${BCTX_FILE}` before proceeding.

---

### Step 3 — Consolidated Report (your only authored output)

Produce:
- **Coverage matrix**: artifact # x written (✅/❌) for selected profile + index + summary.
- **Bounded-context status**: written path, placement check (outside `docs/archetypes/`), and
  validation pass/fail from Step 2.6.
- **Parity table** from Check A.
- **Gap summary**: unresolved high/critical items from the Gap Register.
- **GO / NO-GO** for this service.
- SE canary marker: `🐤 SE-CANARY-OK`

---

## Appendix — EVIDENCE_FILE Template

Create this file at `EVIDENCE_FILE` only if it does not already exist.

```markdown
# Archetype Generation — Shared Evidence & Gap Register
# Service: ${SERVICE_NAME}
# MS Root: ${MS_ROOT_PATH}

> Single source of truth for this service's profile-driven archetype-pack run, split across
> 5 sequential batches. Each batch subagent MUST read this file before starting and append
> its findings when done. Append only — never delete or rewrite a prior batch's section.

## Ingress Inventory (deterministic, orchestrator-run)
| route / queue name | class.method | kind | active (Y/N) | source file |
|--------------------|-------------|------|--------------|------------|
_(populated by orchestrator in Step 0.5 before Batch 1 runs)_

## Batch 1 — Identity & Foundation
_(pending)_

## Batch 2 — Design & Flow
_(pending)_

## Batch 3 — Failure & Governance
_(pending)_

## Batch 4 — Quality, Security & Operations
_(pending)_

## Batch 5 — Close-out
_(pending)_

## Batch 6 — Bounded Context
_(pending)_

## Gap Register (append-only, cumulative)
| Batch | Open Question / Unverified Assumption | Severity | Status |
|-------|--------------------------------------|----------|--------|
```

---

## Orchestrator Change Summary (2026-08-18)

### New mode argument
- Added input `ARCHETYPE_PROFILE` with allowed values: `FULL` or `LEAN`.
- Default profile is `LEAN` when omitted.

### Why this change
- Reduce context load for review and generation runs while preserving major coverage.
- Add bounded-context generation as a first-class deliverable in the same orchestrated run.

### LEAN profile: necessary archetypes kept as standalone
- Kept standalone: `00-IDENTITY-RECONCILIATION.md`
- Kept standalone: `02-SERVICE-MANIFEST.yaml`

### LEAN profile: archetypes merged
- `01-SERVICE-FOUNDATION.md` merges `01-README.md` + `02-ARCHITECTURE.md` + `04-ADRs/` + `05-component-diagram.md` + `06-sequence-diagrams.md` + `07-data-model.md`
- `03-ASSURANCE-AND-OPERATIONS.md` merges `08-error-taxonomy.md` + `09-tier3-service-rules.md` + `10-service-memories-seed.md` + `11-test-strategy.md` + `12-threat-model.md` + `13-observability-contract.md` + `14-runbook.md` + `15-tribal-knowledge.md`

### Coverage guardrails retained in LEAN mode
- Ingress inventory remains mandatory and deterministic.
- One active ingress path still requires one sequence diagram.
- Flow parity and Mermaid validation are still mandatory before GO.

### Bounded-context integration
- Added Batch 6 to run `prompts/universal-bounded-context.prompt.md` after archetype batches.
- Resolved `BCTX_PROMPT` from the generator workspace prompt folder instead of `${MS_ROOT_PATH}`.
- Output fixed at `docs/BOUNDED-CONTEXT-${SERVICE_NAME}.md` (outside `docs/archetypes/`).
- Added mandatory bounded-context structure and Mermaid validation checks.
