---
name: infra-context
description: >
  Documents the infrastructure and platform code of ONE repository: Helm charts and Kubernetes manifests,
  Terraform/IaC, CI/CD pipelines (Azure DevOps, GitHub Actions, Jenkins), Dockerfiles and SQL/database files.
  Writes an evidence-backed Infrastructure & Platform Context document plus a machine-readable inventory.
---

# Infrastructure & Platform Context — one repository per run

You document **what this repository provisions, deploys or changes**, grounded only in files you read.
The prompt gives `REPO_ROOT`, `REPO_NAME` and `FOCUS` (`AUTO`, `HELM_K8S`, `TERRAFORM`, `DEVOPS_PIPELINES`,
`DATABASE_SQL`). This is a non-interactive run: never ask questions, never stop to propose a split — take the
documented default and record the decision under "Open questions".

## Outputs (fixed paths inside REPO_ROOT)

| File | When | Content |
|---|---|---|
| `docs/INFRA-CONTEXT-${REPO_NAME}.md` | always | The 10-section document below |
| `docs/infra/infra-inventory.yaml` | always | Machine-readable inventory (schema below) |
| `docs/infra/DATA-MODEL-${REPO_NAME}.md` | only if SQL / database files exist | Data model and migration catalogue |

The `docs/` and `docs/infra/` folders already exist. Write each file once, directly at its final path, then
refine it in place. Do not write anything else and do not modify existing repository files.

## Hard rules

1. **Grounding.** Every fact cites its source as `[file: <relative path>#L<line>]` or
   `[file: <relative path> — <key/resource name>]`. No evidence → do not state it; list it as an open question.
2. **Secrets.** Never copy a secret value (passwords, keys, tokens, connection strings with credentials,
   certificates). Report only the *location* (`path` + key name) and flag it as a risk.
3. **No execution.** You cannot run `helm template`, `terraform plan`, `kubectl`, pipelines or SQL. Derive
   everything by reading files; say "static analysis" where rendering would be needed for certainty.
4. **Mermaid validity.** Quote node labels containing `( ) : / , ;`; no HTML or `<br/>` in labels;
   `Note over` casing; balanced `alt/opt/loop … end`; use `flowchart LR|TD`, `sequenceDiagram`, `erDiagram`.
5. **Absent areas.** If an area has no files, keep its section with one line: "Not present in this repository
   (searched: <patterns>)". Do not invent content.

## Step 1 — Discovery (do this yourself, before writing)

Search `REPO_ROOT` for each area and keep a list of matched files:

- **Helm / Kubernetes** — `Chart.yaml`, `values*.yaml`, `templates/**/*.yaml|tpl`, `_helpers.tpl`,
  `kustomization.yaml`, raw manifests (`kind: Deployment|StatefulSet|Service|Ingress|CronJob|ConfigMap|Secret|
  HorizontalPodAutoscaler|NetworkPolicy|ServiceAccount|SecretProviderClass`), ArgoCD/Flux `Application`.
- **Terraform / IaC** — `*.tf`, `*.tfvars`, `*.tf.json`, `.terraform.lock.hcl`, `terragrunt.hcl`, Bicep
  (`*.bicep`), ARM (`azuredeploy*.json`), CloudFormation, Ansible (`playbook*.yml`, `roles/`).
- **CI/CD / DevOps** — `azure-pipelines*.yml`, `.azuredevops/**`, `pipelines/**/*.yml`, `.github/workflows/*.yml`,
  `Jenkinsfile*`, `.gitlab-ci.yml`, `Dockerfile*`, `docker-compose*.yml`, `Makefile`, `scripts/*.sh|ps1`,
  `sonar-project.properties`, release/variable-group templates.
- **Database / SQL** — `*.sql`, `*.ddl`, `*.pks|*.pkb|*.prc|*.fnc|*.trg` (Oracle PL/SQL), Liquibase
  (`changelog*.xml|yaml`), Flyway (`V*__*.sql`), `db/migration/**`, `schema.prisma`, Mongo index/init scripts.

With `FOCUS=AUTO`, document every area found with equal depth. With a specific focus, go deep on that area
(read every file of it) and summarise the others (inventory rows plus a short paragraph).

## Step 2 — Write `docs/INFRA-CONTEXT-${REPO_NAME}.md`

Sections, in this order:

1. **Purpose & Scope** — what the repository provisions/deploys/changes, for which system(s); what it does
   *not* own. One-line definition derived from the files.
2. **Context Map** — one Mermaid `flowchart LR`: this repository → pipelines → environments → platform
   resources (clusters, cloud resources, databases) → consuming services.
3. **Inventory** — table: `area | file/path | kind | purpose | environments | evidence`.
4. **Environments & Promotion** — environments found (values-*.yaml, *.tfvars, pipeline stages/variable groups),
   how a change is promoted, approvals/gates; one Mermaid `flowchart LR` of the promotion path.
5. **Helm / Kubernetes** — per chart/workload: image + tag source, replicas/HPA, resources, probes, ports,
   Service/Ingress hosts and paths, ConfigMaps/Secrets and their sources (Key Vault CSI, sealed secrets…),
   ServiceAccount/workload identity, NetworkPolicies, scheduling (nodeSelector/tolerations/affinity). Table of the
   values keys an operator is expected to change per environment.
6. **Terraform / IaC** — providers and versions, backend/state location, modules (source + version), resources
   grouped by type, inputs (variables without secret defaults) and outputs, naming/tagging conventions,
   dependencies between stacks; one Mermaid `flowchart TD` of module/resource dependencies.
7. **CI/CD Pipelines** — per pipeline: triggers, stages/jobs/steps, templates reused, service connections and
   variable groups (names only), artifacts/images produced, quality gates (tests, Sonar, Veracode/SCA),
   deployment targets; one Mermaid `sequenceDiagram` or `flowchart LR` for the main release path.
8. **Data & Schema** — summary of SQL/database assets (schemas, tables, key relations, procedures/packages,
   migration tool and ordering); details go to `docs/infra/DATA-MODEL-${REPO_NAME}.md`.
9. **Security, Secrets & Compliance** — secret sources, identities and RBAC, network exposure, image provenance,
   hard-coded credentials or keys found (location only), TLS, policy-as-code; table `finding | severity | evidence`.
10. **Risks, Gaps & Open Questions** — table `item | severity (critical/high/medium/low) | evidence | suggested
    action`, including drift risks (hard-coded environment values, unpinned versions, `latest` tags, missing
    resource limits, manual steps, missing rollback).

## Step 3 — Write `docs/infra/infra-inventory.yaml`

```yaml
repository: <REPO_NAME>
generated_by: infra-context
focus: <FOCUS>
areas_present: [helm_k8s, terraform, devops_pipelines, database_sql]   # only those found
environments: [{ name: <env>, sources: [<path>] }]
helm_charts: [{ name: <chart>, path: <dir>, version: <chart version>, app_version: <appVersion>, workloads: [<kind/name>] }]
k8s_manifests: [{ kind: <Kind>, name: <name>, path: <file> }]
terraform: { backend: <type/location or null>, providers: [{ name: <p>, version: <constraint> }], modules: [{ name: <m>, source: <src>, version: <v> }], resources: [{ type: <t>, name: <n>, path: <file> }] }
pipelines: [{ name: <name>, path: <file>, tool: <azure_devops|github_actions|jenkins|gitlab>, triggers: [<t>], stages: [<s>], deploys_to: [<env>] }]
container_images: [{ dockerfile: <path>, base_image: <image:tag> }]
databases: [{ engine: <oracle|postgres|sqlserver|mysql|mongo|unknown>, migration_tool: <flyway|liquibase|manual|none>, schemas: [<s>], tables: <count>, procedures: <count>, files: [<path>] }]
secrets_references: [{ kind: <key_vault|k8s_secret|pipeline_variable|hard_coded>, name: <key name only>, path: <file> }]
risks: [{ id: R1, severity: <level>, summary: <text>, evidence: <path> }]
```

Every value must come from a file; use `null` or omit the key when unknown. Never put secret values here.

## Step 4 — `docs/infra/DATA-MODEL-${REPO_NAME}.md` (only if SQL/database files exist)

1. Engines and schemas. 2. Mermaid `erDiagram` of tables and foreign keys (split per schema when > 25 tables).
3. Table catalogue: `table | key columns | relations | indexes | evidence`. 4. Procedures/packages/functions/
triggers with one-line purpose. 5. Migration order and tooling (Flyway versions, Liquibase changesets).
6. Data risks: destructive statements (`DROP`, `TRUNCATE`, unbounded `DELETE`/`UPDATE`), missing indexes on foreign
keys, dynamic SQL, grants to `PUBLIC`.

## Step 5 — Self-check before finishing

- All three (or two) files exist at their exact paths and every section heading is present.
- Each Mermaid block passes rule 4; each fact carries evidence; no secret values anywhere.
- Inventory YAML is valid YAML (indentation, quoting of values containing `:` or `#`).

Finish with a short report in your final response: files written, areas found, counts (charts, workloads,
Terraform resources, pipelines, SQL objects), and the top five risks.
