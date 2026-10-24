# Per-Batch Subagent Prompt Template

This file is the verbatim prompt template for each `runSubagent` call made by the
orchestrator. The orchestrator substitutes all `${VARIABLE}` placeholders before sending.

---

## Prompt (copy verbatim, substituting placeholders)

You are Batch **${BATCH_NUMBER}** of 5, generating a subset of the AIFC archetype pack for
ONE microservice: **`${SERVICE_NAME}`**.

Selected profile: **`${ARCHETYPE_PROFILE}`** (`FULL` or `LEAN`).

---

### Step 1 — Read before writing

Read `${EVIDENCE_FILE}` in full before doing anything else. It contains:
- Facts established by earlier batches (tech stack, identity, manifest values, findings, open gaps).
- The **Ingress Inventory** table — a deterministic, orchestrator-run grep of every REST
  endpoint (method level), EventHub consumer, ServiceBus receiver, and scheduled trigger in
  the codebase. This is your completeness checklist.

Reuse those facts — do NOT re-derive or contradict them. If you find a fact is wrong, correct
it in your own evidence section and flag it as a correction. Do not silently overwrite history.

**If this batch covers ingress artifacts (FULL `02-ARCHITECTURE` or LEAN `01-SERVICE-FOUNDATION`):**
every row in the Ingress Inventory must be represented in your output, explicitly labeled as
active or inactive. A missing row is a bug — fix it before finishing.

**Inlined Ingress Inventory** (provided by orchestrator for Batches 1 and 2):
```
${INGRESS_INVENTORY_TABLE}
```

---

### Step 2 — Scope and grounding rules

1. Read source, config, and tests ONLY under `${MS_ROOT_PATH}` for any NEW facts this batch
   needs. Do not read from any other microservice folder.
2. Every new claim must trace to a file you actually read. If data is missing, state the
   assumption briefly, log it in the Gap Register, and continue — do not fabricate.
3. Do NOT reference any other microservice.

---

### Step 3 — Generate exactly these artifacts

Write the following files under `${OUTPUT_DIR}` (create the directory if absent), using the
exact file names and per-file required content from the pack specification in
`orchestrate.agent.md`:

```
${BATCH_ARTIFACT_LIST}
```

Interpret `${BATCH_ARTIFACT_LIST}` according to `${ARCHETYPE_PROFILE}`:
- `FULL`: generate standalone artifacts exactly as listed.
- `LEAN`: generate merged artifacts exactly as listed, with explicit section headers that map
   to the canonical standalone artifact names they cover.
- `${ARCHETYPE_PROFILE}` is a mode selector only; never prepend it to any output file name.

---

### Artifact-specific requirements

**If this batch includes `05-component-diagram.md` (FULL):**
- Produce a Mermaid `flowchart` diagram of all internal building blocks and their wiring.
- Every active ingress path from the Ingress Inventory must appear as a node.
- Inactive paths must be present but labeled as inactive/commented-out, not silently omitted.

**If this batch includes `06-sequence-diagrams.md` (FULL):**
- One `sequenceDiagram` per row in the Ingress Inventory that is marked active (Y).
- One diagram per inactive path too, clearly labeled inactive, if space allows; otherwise note
  it in the file header as explicitly excluded with a reason.
- Do NOT consolidate two unrelated ingress paths into one diagram.
- Every diagram must have a source attribution comment (file path(s) it was derived from).

**If this batch includes `01-SERVICE-FOUNDATION.md` (LEAN):**
- Include these top-level sections in order:
   1) `## Service Overview and Boundaries (from 01-README and 02-ARCHITECTURE)`
   2) `## ADR Decisions (from 04-ADRs)`
   3) `## Component Diagram (from 05-component-diagram)`
   4) `## Sequence Diagrams (from 06-sequence-diagrams)`
   5) `## Data Model (from 07-data-model)`
- In `Component Diagram` section: produce a Mermaid `flowchart` with active and inactive ingress.
- In `Sequence Diagrams` section: one `sequenceDiagram` per active ingress row; no unrelated-flow
   consolidation in a single diagram.
- Inactive paths must be explicitly called out as inactive (diagram or exclusion note).

**If this batch includes `02-ARCHITECTURE.md` (FULL) or `01-SERVICE-FOUNDATION.md` (LEAN):**
- The Ingress table must have one row per Ingress Inventory entry.
- Active/inactive status must match the inventory — no stale rows.

**If this batch includes `12-threat-model.md` (FULL) or the threat-model section in
`03-ASSURANCE-AND-OPERATIONS.md` (LEAN):**
- Include a control/trust-boundary Mermaid diagram.

**If this batch includes `14-runbook.md` (FULL) or the runbook section in
`03-ASSURANCE-AND-OPERATIONS.md` (LEAN):**
- Include a triage flow Mermaid diagram + startup checklist.

**If this batch includes `03-ASSURANCE-AND-OPERATIONS.md` (LEAN):**
- Include these top-level sections:
   1) `## Error Taxonomy (from 08-error-taxonomy)`
   2) `## Tier-3 Service Rules (from 09-tier3-service-rules)`
   3) `## Service Memories Seed (from 10-service-memories-seed)`
   4) `## Test Strategy (from 11-test-strategy)`
   5) `## Threat Model (from 12-threat-model)`
   6) `## Observability Contract (from 13-observability-contract)`
   7) `## Runbook (from 14-runbook)`
   8) `## Tribal Knowledge (from 15-tribal-knowledge)`

**If this is Batch 5 (includes `00-INDEX.md` and `ARCHETYPE-SUMMARY.md`):**
- Read the entire evidence file.
- Synthesize a consolidated gap register across all 5 batches.
- Write `00-INDEX.md` (artifact index + open gaps) and `ARCHETYPE-SUMMARY.md`
   (one-line summary per artifact + consolidated gap register).
- In `ARCHETYPE-SUMMARY.md`, include a profile note:
   - `FULL`: state all 16 archetypes were generated.
   - `LEAN`: state kept standalone vs merged archetypes, and list merge mapping.

---

### Mermaid rendering self-check (required for any batch producing a diagram)

Before finishing, scan every Mermaid code block you wrote for these parse-breaking patterns
and fix any you find — do not defer to a later pass:

1. **Unquoted special-character aliases** — `participant X as Foo (Bar)` must be
   `participant X as "Foo (Bar)"`. Same for node labels in flowcharts containing `(`, `)`,
   `:`, `/`.
2. **Semicolons in note/label text** — `Note over X: foo; bar` must be
   `Note over X: foo, bar` (Mermaid treats `;` as a statement terminator).
3. **HTML tags in labels** — `<br/>` inside labels → replace with a space or comma.
4. **Keyword casing** — `note over` → `Note over`.
5. **Unbalanced blocks** — every `alt`/`opt`/`loop` must have a matching `end`.

---

### Quality checks before finishing

- `FULL` parity check: if this batch includes 05 and/or 06, build a parity table of every
   distinct ingress path in `05-component-diagram.md` vs its corresponding diagram in
   `06-sequence-diagrams.md`. Every path must appear in both, with no consolidation of unrelated
   flows; fix any gap directly before finishing.
- `LEAN` parity check: if this batch includes `01-SERVICE-FOUNDATION.md`, build a parity table of
   every distinct ingress path in the `Component Diagram` section vs its corresponding diagram in
   the `Sequence Diagrams` section. Same no-consolidation rule applies.
- No cross-microservice references anywhere in the artifacts.
- Manifest/schema consistency with earlier batches.
- Every open gap is code-traceable (cites a file path).
- Concise but concrete — no filler prose.

---

### MANDATORY final step — append to `${EVIDENCE_FILE}`

Append under `## Batch ${BATCH_NUMBER} — <short name>` (append-only; never delete or edit
prior sections):

```markdown
### Key files read
- <list>

### Facts established / corrected
- <list; mark corrections explicitly>

### Artifacts written
- <exact paths>

### Ingress coverage (Batches 1 & 2 only)
- Inventory rows covered: N / N
- Any row not covered: <reason>
```

Add one Gap Register row per unresolved high/critical item or unverified assumption:

```markdown
| ${BATCH_NUMBER} | <description> | high/medium/low | open |
```

---

### Return value

Return a short summary:
- Artifact paths written
- Ingress Inventory rows covered (if batch 1 or 2)
- Count of new open gaps added
