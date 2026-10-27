---
name: omni-rule-workflow-compliance-attestation
description: 'Apply when completing any OMNI workflow (/omni-plan, /omni-scaffold, /omni-test, /omni-debug, /omni-refactor, /omni-document) or gate to enforce visible compliance attestation and canary rule verification Invoke when an omni-* skill mandates this rule.'
metadata:
  author: "OMNI AIFC Team <omni-aifc@att.com>"
keywords: [rule, workflow, compliance, attestation]
layer: OMNI-RULE
---

# omni-rule-workflow-compliance-attestation

> **Type**: OMNI rule sub-skill (atomic). Invoked by parent `omni-*` skills via the `skill` tool when a workflow mandates this rule. Not user-facing — do not route user prompts here.
>
> **Source rule**: `.devin/rules/workflow-compliance-attestation.md` — apply the content below in full and verbatim.

---

# Workflow Compliance Attestation & Canary Rules

> **Tier**: OMNI Rules | **Version**: 1.0.0 | **Applies To**: All OMNI workflows and gates
>
> **Purpose**: Ensure rule compliance is **visible and verifiable** by all developers, not just experts.

---

## 🔥 HARD-STOPS

- ✘ **No workflow completion without attestation footer**: Every OMNI workflow must end with a visible compliance attestation block
- ✘ **No silent rule skipping**: If a rule or directive was skipped, the attestation must explicitly list it with a reason
- ✘ **No canary omission**: The canary marker must appear in every attestation — its absence signals rule system failure

---

## A1. Compliance Attestation Footer (MANDATORY)

Every OMNI loop workflow (`/omni-plan`, `/omni-scaffold`, `/omni-test`, `/omni-debug`, `/omni-refactor`, `/omni-document`) and every gate workflow (`/omni-gate-*`) **MUST** end its output with the following attestation block:

```markdown
---
### OMNI Compliance Attestation
| Item | Status |
|------|--------|
| **Workflow** | {workflow name and mode} |
| **Rules loaded** | {comma-separated list of .devin/rules/ files consulted} |
| **Domain rules** | {domain rules loaded, or "None — no domain repo in workspace"} |
| **Hard-stops checked** | {count of hard-stops verified} |
| **Directives followed** | {count of workflow directives executed} |
| **Directives skipped** | {list of skipped directives with reasons, or "None"} |
| **Canary** | 🐤 OMNI-CANARY-OK |
---
```

### Why This Matters

- **For junior developers**: If you don't see `🐤 OMNI-CANARY-OK` at the end of a workflow output, the rule system did not fully engage. Flag this to your tech lead.
- **For tech leads**: Review the "Directives skipped" row. If rules are being silently dropped, the workflow or rule file may need investigation.
- **For the OMNI team**: Track canary failures across sessions to identify systemic rule-loading issues.

---

## A2. Canary Rule

This is a deliberate verification marker. The AI **MUST** include `🐤 OMNI-CANARY-OK` in the attestation footer of every workflow completion.

**If this marker is missing**, it means one of:
1. The rule file was not loaded into context
2. The workflow did not reach its completion step
3. The AI deprioritized or truncated the attestation

**Action on missing canary**:
- Developer: Re-run the workflow and check if the canary appears
- If it fails twice: Report to the OMNI team with the workflow name and session context
- OMNI team: Investigate whether the rule file or workflow exceeded context limits

---

## A3. Rule Placement Guidance

To maximize the chance that rules are followed:

| Location | Loading Behavior | Best For |
|----------|-----------------|----------|
| `.devin/rules/*.md` | Auto-loaded by trigger condition | Hard-stops, always-on standards |
| Workflow `.md` (top 200 lines) | Read when workflow is invoked | Critical workflow directives |
| Workflow `.md` (bottom of file) | May be deprioritized in long files | Low-priority checklists |
| Persistent memory (via `/omni-seed-memories`) | Loaded every session | Critical cross-session rules |

**Recommendation**: Keep critical directives in rules files or the top portion of workflows. Move self-validation checklists to separate, shorter files if they exceed 50 items.

---

## A4. Workflow Size Budget

To reduce the risk of rules being ignored due to context window limitations:

| Workflow Size | Risk Level | Action |
|--------------|-----------|--------|
| < 300 lines | Low | No action needed |
| 300–500 lines | Medium | Consider splitting modes into sub-files |
| 500–800 lines | High | Split required — extract modes to separate workflows |
| > 800 lines | Critical | Must split immediately — rules at the bottom will be ignored |

---

## Change Log

| Version | Date | Summary |
|---------|------|---------|
| 1.0.0 | 2026-04-08 | Initial creation — attestation footer, canary rule, placement guidance, size budget |
