# Agent bundle

The worker runs whatever agent is in this directory through GitHub Copilot CLI. Nothing about the agent's
behaviour is hard-coded in the platform: the agent file is copied verbatim into a per-job `COPILOT_HOME`, the
prompt comes from `bundle.yaml`, and only files matching `output.paths` are ever committed.

**The agent currently here is a placeholder** (`placeholder: true`). The UI shows a warning while it is active.

## Installing the real agent

1. Copy the real agent file(s) into `agents/` unchanged, e.g.
   `cp <repo>/.github/agents/BoundedContext.agent.md agent-bundle/agents/`.
   Copy any files the agent references with `#file:` or relative links next to it, preserving relative paths.
   Delete `agents/bounded-context.agent.md`.
2. Edit `bundle.yaml`:
   - `agent.id` → the new file name without `.agent.md` (e.g. `BoundedContext`).
   - `prompt.template` → the exact prompt developers type in VS Code. Keep `{{output_path}}` only if the
     prompt in VS Code names the output file; otherwise remove it.
   - `output.paths` / `output.primary` → where the agent writes (glob patterns allowed, e.g.
     `docs/bounded-contexts/*.md`). Leave `primary` out if the agent picks file names itself.
   - `inputs` → only values the agent truly cannot discover itself. Delete the placeholder `scope_path`
     input if the real workflow has no equivalent.
   - `runtime.allow_tools` / `deny_tools` → keep `shell` and `url` denied unless the agent cannot work without
     them; record the reason in a comment. If the agent uses MCP servers, add an `mcp.json` here and set
     `runtime.mcp_config` — servers that need corporate credentials must have them supplied by Key Vault.
   - `placeholder: false`.
3. Check the bundle offline:
   ```bash
   cargo run -p bcp-agent --bin bcp-agent-run -- validate --bundle agent-bundle
   ```
4. Run it against a local checkout with your own Copilot access (no web app needed):
   ```bash
   export COPILOT_GITHUB_TOKEN=<fine-grained PAT with "Copilot Requests", or `gh auth token`>
   cargo run -p bcp-agent --bin bcp-agent-run -- run --bundle agent-bundle \
       --workspace /path/to/checkout --model auto --owner acme --repo shop --branch main
   ```
   Compare the produced file with what the agent produces in VS Code for the same commit.
5. Rebuild the worker image (the bundle is baked in) or update the `agent-bundle` ConfigMap in Helm
   (`worker.agentBundle.source: configmap`).

## Things that behave differently outside VS Code

| VS Code feature | Under Copilot CLI | Action |
|---|---|---|
| VS Code tool ids in `tools:` (`execute/runInTerminal`, `edit/createFile`, `search/codebase`, `vscode`) | Unknown names are ignored; CLI built-ins `read`, `edit`, `search`, `execute` are used instead | Nothing, unless the list contains *only* VS Code ids — then add CLI aliases (`read`, `edit`, `search`) |
| `#file:` references | Resolved relative to the agent file | Ship referenced files in the bundle |
| MCP servers configured in VS Code | Not available | Add `mcp.json` + `runtime.mcp_config` |
| `model:` in frontmatter | Overrides the model chosen in the UI | The portal locks the model selector and says why |
| Interactive follow-up questions | Disabled (`--no-ask-user`) | Put required answers in `inputs` |

## Generator repositories (bundles outside the image)

Besides this built-in bundle, the portal can run a bundle that lives in a GitHub repository (see
`config/generators.yaml` and `docs/architecture.md`). Such a repository needs the same two things as this
directory, at the repository root or in a sub-directory (`path` when adding it):

```
<path>/bundle.yaml            # the manifest described above
<path>/agents/<id>.agent.md   # the agent files, verbatim (or beside the manifest, see agents_dir)
```

If your agent already lives in a folder such as `aifc-archetype-generator/orchestrate.agent.md` with prompt files
next to it, put `bundle.yaml` in that folder and add `agents_dir: "."` at its top level, so the existing layout
is copied unchanged into the agent home.

Bundles loaded from repositories always run under the portal's hard limits, regardless of `runtime:` — shell,
url and memory tools are denied, MCP servers are not available, repository instructions are off — and
`output.paths` must be documentation files (`md`, `mdx`, `txt`, `yaml`, `yml`, `json`) below a `docs` directory
(`docs/*.md`, `**/docs/archetypes/*.yaml`, …). A bundle that asks for more (MCP servers, `.github/**`, source
files) is rejected with `generator_invalid` and an explanation.
