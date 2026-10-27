## Mode: --readme (README & AGENTS.md Generation)

> **Absorbs**: `omni-plan-readme.md`
> **Template**: `templates/OMNI-README-TEMPLATE.md`

### Step 1: Collect Target Repository Input

1. Ask the requestor to provide either:
   - **Repository name** in the current workspace, or
   - **Repository path** in the current workspace.
2. Ask whether the requestor wants to provide a **README template path** for additional guidance.
3. If the requestor provides a template path, validate that it exists and read it as either:
   - a **custom baseline** if it clearly replaces the default structure, or
   - an **overlay/reference** if it is domain-specific guidance layered on top of the Omni standard.
4. If the requestor does not provide a template path, use the default template from `templates/OMNI-README-TEMPLATE.md` as the canonical baseline.
5. If the requestor provides only a repository name, resolve it to the matching workspace path before continuing.
6. Halt and ask for clarification if the repository cannot be located in the workspace.
7. Confirm the final target repository root before creating documentation files.

### Step 2: Inspect the Target Repository (Per-Directory Discovery)

> **CRITICAL**: Do NOT use a single recursive search or `find_by_name` with unlimited depth to discover the project tree. You MUST walk the tree level-by-level using `list_dir` on each subdirectory individually.

1. Run `list_dir` on the **repository root** to enumerate top-level files and directories.
2. For **every subdirectory** returned, run a separate `list_dir` call to enumerate its contents. Continue descending until all non-ignored paths have been visited.
   - Batch independent `list_dir` calls in parallel where possible.
   - Skip `.git/`, `target/`, `build/`, `node_modules/`, and other standard ignored directories.
3. Record: total file count per directory, source directories, test directories, config/deployment/docs folders, build files.
4. Assemble the full project directory structure in **ASCII-tree form**.
5. Identify component type, runtime, language, key entry points, major integrations, and build/test commands.

### Verification Gate (Mandatory)

6. **Cross-check file counts**: Sum the file counts across all directories. Compare this total against a top-level `find_by_name` with `Pattern: "*"` and `Type: "file"`.
   - If the totals differ by more than 5%, re-walk the directories that were skipped or under-counted.
   - Do not proceed to Step 3 until the file count is verified.

### Step 3: Domain Overlay Discovery

1. Check if the target repo's domain has a README overlay in the domain team-documents repo:
   - Pattern: `{domain-team-documents}/templates/{domain}/README-TEMPLATE-*-OVERLAY.md`
2. If an overlay exists, read and apply it as an additive layer on top of the OMNI baseline.

### Step 4: Generate README.md and AGENTS.md (Atomic)

> ⚠️ **GATE**: Both `README.md` and `AGENTS.md` MUST be generated in this step. Do NOT proceed to Step 5 until both files have been written to the repository root.

#### 4a. Generate README.md

1. Using the OMNI template + domain overlay (if found) + repository inspection results:
   - Generate all required sections (Overview, Architecture, API Endpoints, Workers table if OCE, Build/Test, Configuration, Change Log)
   - Include Mermaid architecture diagram
   - Include endpoints table for REST services
   - Include change log with current date
2. Write `README.md` to the repository root using `write_to_file` or `edit`.

#### 4b. Generate AGENTS.md

> **MANDATORY**: This file MUST be created whenever README.md is created. It is not optional.

1. Create `AGENTS.md` at the repository root with the following sections:
   - **Build Commands**: compile, test, lint, deploy commands (copy-pastable)
   - **Key File Paths**: source roots, config locations, test directories, deployment manifests
   - **Coding Conventions**: language version, framework patterns, naming conventions, hard-stops from domain rules
   - **Domain-Specific Patterns**: worker lifecycle, DMN routing, schema conventions, or other domain rules (if applicable)
   - **Integration Points**: downstream dependencies, Kafka topics, database connections, external APIs
   - **Testing**: test framework, test data locations, coverage commands, minimum coverage thresholds
2. Write `AGENTS.md` to the repository root using `write_to_file` or `edit`.
3. Verify both files exist at the repository root before proceeding.

### Step 5: Validate

1. Verify README.md has YAML frontmatter (`title`, `version`, `last_updated`, `author`).
2. Verify all sections from the template are present.
3. Verify Mermaid diagrams render correctly (valid syntax).
4. **Verify `AGENTS.md` exists** at the repository root.
   - **FAIL** if `AGENTS.md` was not created — return to Step 4b and generate it.
5. Verify `AGENTS.md` contains at minimum: Build Commands, Key File Paths, and Testing sections.

---
