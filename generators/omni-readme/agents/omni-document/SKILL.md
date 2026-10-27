---
name: omni-document
description: 'AIFC Core Loop Step 6: Generate or update documentation - README, API docs, architecture notes, AID. Modes via --mode (registry: document.config.yml): readme, docs, aid, default. Triggers on "omni-document", "/omni-document", or a documentation request.'
metadata:
  author: "OMNI AIFC Team <omni-aifc@att.com>"
keywords: [document, readme, docs, aid, changelog]
layer: AIFC-LOOP
---

# /document — Capture What Matters

> **AIFC Core Loop Step 6** | Use when you need a concise artifact: README, runbook, decision record.
>
> **Modes** (via `--mode <name>`; registry: `document.config.yml`): `--readme`, `--docs`, `--aid`, (default: general documentation)
>
> **Archetype-aware**: If an archetype is detected, routes to the specialist (e.g., `/document-domain-service-java`).
>
> **Provide**: Target audience, what to include/exclude, the file destination.

User input: $ARGUMENTS

---

## Step 0: Mode Dispatch (config-driven)

1. **Check for archetype context**:
   - Search for directory `00-core-orchestration` in the workspace
   - If found, run `python ${ARCHETYPES_BASEDIR}/00-core-orchestration/scripts/discover-archetype.py --query "$ARGUMENTS" --json`
   - If archetype match score ≥ 30 → route to `/{archetype-document-workflow}` and stop
   - If score 15–29 → present match and ask user to confirm or continue with OMNI-general
   - If score < 15 or no archetype dir → continue with OMNI-general steps below

2. **Resolve mode** via `document.config.yml` (this directory) -> `mode_selection` + `modes:` registry — the single source of truth for modes. Explicit `--mode <name>` always wins; else match `modes.*.triggers` keyword triggers, then `modes.*.arg_patterns`, then `mode_selection.default`. **Adding or retuning a mode = a config entry + a `references/mode-*.md` file — never edit this dispatch.**

3. **If no explicit mode is specified by the user**:
   > ⚠️ **GATE**: Do NOT proceed until all mode options are acknowledged via `ask_user_question`.
   >
   > Present these options and let the user choose before continuing:
   > - `--readme` — Generate or refresh README and AGENTS.md style project documentation.
   > - `--docs` — Validate and synchronize documentation quality, frontmatter, diagrams, and change logs.
   > - `--aid` — Generate an Application Interface Document (AID) from SI, TDD, wiki, or other source documents.
   > - `default` — Create a general documentation artifact for the requested audience and scope.
   >
   > Use `ask_user_question` to present the mode options with one-line descriptions, then continue in the selected mode.

---
4. **Execute**: load `references/mode-<resolved>.md` (per `modes.<name>.reference`) and run its procedure verbatim. Mode docs own their pipelines — this file never inlines them. Entries with `modes.*.redirect` hand off to the named skill.

---

## Compliance Attestation (MANDATORY)

> **Enforces**: `omni-rule-workflow-compliance-attestation` skill

After completing any mode in this workflow, the AI **MUST** append the OMNI Compliance Attestation footer. See `omni-rule-workflow-compliance-attestation` skill § A1` for the required format. The `🐤 OMNI-CANARY-OK` marker must be present — its absence signals rule system failure.

---



---

## 🔥 HARD-STOPS

> These are NON-NEGOTIABLE. Violation = HALT the workflow and get explicit user approval before continuing.

- ✘ **Never fabricate data**: Do not invent credentials, secrets, ticket IDs, commit SHAs, release metadata, or attestation evidence.
- ✘ **Never bypass rule files**: If a governing `omni-rule-*` sub-skill (see `.apm/skills/omni-rule-*`) is violated, invoke it, surface the violation, and stop until it is resolved.
- ✘ **Never skip the compliance attestation footer**: Every workflow execution MUST end with the OMNI Compliance Attestation footer and the `🐤 OMNI-CANARY-OK` marker per `omni-rule-workflow-compliance-attestation` skill.
- ✘ **Never invoke inline scripts**: Use only the documented reusable scripts in `scripts/` or the `script-execution-standards.md` `_run()` wrapper; do not embed inline Python/shell in responses.
- ✘ **Never run destructive commands silently**: Any command that modifies, deploys, or releases code must be visible to the user and confirmed when required by this workflow.

---

## ⛔ Common Anti-Patterns

> Avoid these patterns when executing this workflow.

- Generating boilerplate or implementation without first reading the relevant rule files this workflow enforces.
- Skipping user-confirmation gates (`ask_user_question`) when the workflow explicitly requires them.
- Returning partial output without appending the mandatory compliance attestation footer.
- Producing new artifacts without verifying that they resolve the original user input or gate criteria.
- Reusing stale context from a previous session without re-reading the current workspace state.

## References

- **AIFC Workflow Guide**: `plugins/omni-plugin/README.md` (this package's own docs; the legacy `omni-global-rules` repo checkout is not required for APM-installed consumers)
- **Previous step**: [/omni-refactor](../omni-refactor/SKILL.md)
- **Next in cycle**: Back to [/omni-compare](../omni-compare/SKILL.md) or run the packaged `omni-gate-*` skills.
