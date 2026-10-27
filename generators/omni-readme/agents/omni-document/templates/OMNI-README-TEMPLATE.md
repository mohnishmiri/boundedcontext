# Omni Component README — Standardized Template

> **Purpose**: This document defines the **standard README.md template** for Omni platform repositories.
> It is intended for microservices, shared libraries, event processors, UI-backed components,
> batch jobs, adapters, and other platform components so documentation stays consistent,
> navigable, and useful for onboarding, support, and AI-assisted development.

---

## Template Structure

Every Omni component `README.md` should contain the following sections, with conditional sections included only when relevant.

### 1. Title & Metadata Table

```markdown
# {ComponentName} — {Short Component Description}

| Field | Value |
|---|---|
| **Domain / Team** | `{Domain or Team Name}` |
| **Artifact ID** | `{ArtifactId}` |
| **Version** | `{Version}` |
| **Component Type** | `{REST API / REST+Kafka / Kafka Processor / Batch / Shared Library / UI + REST / Adapter / Other}` |
| **Runtime / Parent** | `{sdk-java-parent / sdk-java-library-parent / Spring Boot / Node / other}` |
| **Language / Java Version** | `{Java 17 / Java 11 / TypeScript / Other}` |
| **Base Path / Entry Point** | `{REST base path, topic, batch trigger, UI route, package root, or N/A}` |
| **API / Contract Docs** | `{Swagger UI / OpenAPI JSON / GraphQL schema / WSDL / N/A}` |
```

**Rules**:
- Title uses a `#` heading with the component name and a short description.
- Metadata should reflect the actual component type and runtime rather than assuming one platform pattern.

---

### 2. Table of Contents

```markdown
## Table of Contents

1. [Overview](#overview)
2. [Architecture](#architecture)
3. [Project Structure](#project-structure)
4. [Domain Capability Map](#domain-capability-map)
5. [Integration Context](#integration-context)
6. [Process Flows](#process-flows)
7. [Interfaces & Endpoints](#interfaces--endpoints)
8. [Data Stores & State Management](#data-stores--state-management)
9. [Configuration](#configuration)
10. [Dependencies](#dependencies)
11. [Observability and Operations](#observability-and-operations)
12. [Security and Compliance](#security-and-compliance)
13. [Build and Test](#build-and-test)
14. [Deployment](#deployment)
15. [Change Log / History](#change-log--history)
```

**Rules**:
- Numbered list with anchor links.
- Conditional sections may be omitted if not relevant, but the final README should preserve clear ordering and contiguous numbering.

---

### 3. Overview

```markdown
## Overview

**{ComponentName}** is an Omni platform component responsible for **{primary responsibility}**.
It acts as {role description}, handling:

- **{Capability 1}**: {Brief description}
- **{Capability 2}**: {Brief description}
- ...

This component uses `{key patterns}` such as REST APIs, event-driven processing, shared-library reuse,
batch scheduling, workflow orchestration, UI flows, caching, or adapter-based integration where applicable.
```

**Rules**:
- Start with a bold component name and a one-sentence summary of its role.
- List ALL key capabilities as bullet points.
- Mention the key architectural patterns actually used in the repository.

---

### 4. Architecture

```markdown
## Architecture

\```mermaid
graph TD
    subgraph EntryPoints[...]
        ...
    end
    subgraph {ComponentName}[...]
        subgraph Interface_Layer[...]
            ...
        end
        subgraph Core_Layer[...]
            ...
        end
        ...
    end
    subgraph Dependencies[...]
        ...
    end
\```
```

**Rules**:
- Use **Mermaid `graph TD`** (top-down) for the architecture diagram.
- Show the most relevant layers for the component, such as entry points, component internals, dependencies, and data stores.
- List key architectural patterns after the diagram.

---

### 5. Project Structure

```markdown
## Project Structure

\```
{repo-name}/
├── pom.xml / build.gradle / package.json
├── src/
│   ├── main/
│   │   ├── java|kotlin|ts/...     # application code
│   │   ├── resources/...          # configs, schemas, templates
│   │   └── web/...                # UI assets if applicable
│   └── test/...                   # unit, component, integration tests
├── config/ / opt/ / helm/ / docs/
└── README.md
\```
```

**Rules**:
- Use ASCII tree format.
- Include inline comments (`# purpose`) for key directories.
- Show actual file names for critical files where useful.

---

### 6. Domain Capability Map

```markdown
## Domain Capability Map

| Capability | Description | Primary Interfaces / Entry Points |
|------------|-------------|-----------------------------------|
| **{Capability 1}** | {Description} | `{endpoint/topic/job}` |
| **{Capability 2}** | {Description} | `{endpoint/topic/job}` |
```

**Rules**:
- Summarize the business/technical capabilities owned by the component.
- If the component is a shared library, list the reusable capabilities it exposes.

---

### 7. Integration Context

```markdown
## Integration Context

| Interface / Dependency | Direction | Type | Purpose | Notes |
|------------------------|-----------|------|---------|-------|
| `{caller/system}` | Inbound | `{REST / event / file / UI / library}` | {Purpose} | {Notes} |
| `{downstream/system}` | Outbound | `{REST / Kafka / DB / cache / library}` | {Purpose} | {Notes} |
```

**Rules**:
- Make upstream and downstream relationships explicit.
- Include shared libraries and data stores when they materially affect behavior.

---

### 8. Process Flows

```markdown
## Process Flows

### 1. {Flow Name}

\```mermaid
sequenceDiagram
    participant Caller as {Caller}
    participant Interface as {Controller/Consumer/Job}
    participant Core as {Service/Processor}
    participant Dependency as {Dependency}
    participant Store as {DataStore}
    ...
\```

| Input | Output |
|-------|--------|
| `{Input}` ({key input fields}) | {Description of output / side effect / response} |

> **Code Reference**: `{ClassName}.{methodName}()` → `{NextClass}.{method}()`
```

**Rules**:
- Use **Mermaid `sequenceDiagram`** for each process flow.
- Include the major runtime paths relevant to the component (REST, event, batch, scheduler, UI, library call, lifecycle).
- Each diagram should have an **Input / Output** table immediately after it.
- Each diagram must have a `> **Code Reference**` callout linking to the implementing classes.

---

### 9. Interfaces & Endpoints

```markdown
## Interfaces & Endpoints

### REST / GraphQL / SOAP / UI Routes *(conditional)*

| # | Interface | Method / Trigger | Path / Topic / Job | Description | Code Reference |
|---|-----------|------------------|--------------------|-------------|----------------|
| 1 | `{REST / Event / Batch / UI / Library}` | `{POST / consumer / cron / public method}` | `{path/topic/job}` | {Description} | `{Class.method()}` |

### Downstream Service Integrations *(conditional)*

| # | Dependency | Protocol | Backend Resource Endpoint URL | Purpose | Code Reference |
|---|------------|----------|-------------------------------|---------|----------------|
| 1 | `{Downstream Service}` | `{REST / GraphQL / SOAP / gRPC / client}` | `/msapi/{service}/v{version}/{resource}` | {Purpose} | `{ClientClass.method()}` |
```

**Rules**:
- Document the component's externally observable interfaces.
- Document downstream integrations here when the component calls external services or platform dependencies.
- Include `Backend Resource Endpoint URL` as the full path (base URL + resource endpoint URL). Do not include scheme, host, or port. Example: `/msapi/billms/v1/bills`.
- Add common headers, error responses, event metadata, or trigger notes where relevant.

---

### 10. Data Stores & State Management

```markdown
## Data Stores & State Management

| Store / State | Type | Purpose | Access Pattern | Notes |
|---------------|------|---------|----------------|-------|
| `{PostgreSQL / Cassandra / Redis / Cosmos / file store / in-memory}` | `{DB / Cache / File / N/A}` | {Purpose} | `{DAO / repository / cache / direct client}` | {Notes} |
```

**Rules**:
- Include only the stores and stateful mechanisms that matter to runtime behavior.
- Shared libraries may use this section to document managed models, DAOs, or caches.

---

### 11. Configuration

```markdown
## Configuration

| Setting Group | File / Source | Purpose |
|---------------|---------------|---------|
| **Application Properties** | `application.properties` | Core runtime configuration |
| **Feature Flags** | `{Azure App Config or Spring Config Server/ LaunchDarkly / local config / N/A}` | Controlled rollout |
| **Secrets** | `{Azure Key Vault or AWS Secrets Manager / env vars / loader / N/A}` | Sensitive settings |
```

**Rules**:
- Document only the important configuration groups and their purpose.
- Never include real secrets.

---

### 12. Dependencies

```markdown
## Dependencies

### Runtime / Parent
| Dependency | Version | Purpose |
|------------|---------|---------|
| `{parent/runtime}` | `{version}` | {Purpose} |

### Key Dependencies
| Dependency | Version | Purpose |
|------------|---------|---------|
| `{dependency}` | `{version}` | {Purpose} |
```

---

### 13. Observability and Operations

```markdown
## Observability and Operations

| Area | Mechanism | Notes |
|------|-----------|-------|
| **Logging** | `{framework / sink}` | {Notes} |
| **Metrics** | `{Micrometer / Prometheus / custom / N/A}` | {Notes} |
| **Tracing / Correlation** | `{trace header / correlation ID}` | {Notes} |
| **Operational Tasks** | `{replay / retry / scheduler / health check}` | {Notes} |
```

**Rules**:
- Explain how operators and developers observe, support, and troubleshoot the component.

---

### 14. Security and Compliance

```markdown
## Security and Compliance

| Area | Approach | Notes |
|------|----------|-------|
| **Authentication / Authorization** | `{OAuth2 / AAF / Azure AD / internal / N/A}` | {Notes} |
| **Sensitive Data Handling** | `{masking / encryption / exclusion}` | {Notes} |
| **Secrets Management** | `{Azure Key Vault / AWS Secrets Manager / env vars / loader}` | {Notes} |
| **Compliance Considerations** | `{PII / audit / retention / SOX / other}` | {Notes} |
```

---

### 15. Build and Test

```markdown
## Build and Test

### Build
\```bash
{repo build command}
\```

### Run Tests
\```bash
{repo test commands}
\```

### Profiles
| Profile | Description |
|---------|-------------|

### Code Coverage
- **Coverage Report**: `{path}`
- **Quality Dashboard**: `{URL or tool}`
```

---

### 16. Deployment

```markdown
## Deployment

### Build Artifact / Image
\```bash
{build or image command}
\```

### Pipeline
- `{pipeline URL}`
- `{deployment docs URL}`
- `{onboarding or environment docs URL}`

### Runtime Notes
| Area | Value |
|------|-------|
| **Deployment Target** | `{k8s / VM / function / library release / N/A}` |
| **Environment Notes** | `{notes}` |
```

---

### 17. Change Log / History

```markdown
## Change Log / History

| Ticket | Description |
|--------|-------------|
| **XXXXXXXXX-XXXXX** | Description of change |
```

**Rules**:
- Most recent changes first.
- Include work item IDs and concise descriptions.

---

## Formatting Standards

| Rule | Standard |
|------|----------|
| **Headings** | `##` for main sections, `###` for subsections |
| **Diagrams** | Mermaid only (`graph TD`, `sequenceDiagram`) |
| **Tables** | Pipe-delimited Markdown tables with header separator |
| **Code blocks** | Fenced with language hint (`java`,`groovy`, `bash`, `properties`, `xml`, `mermaid`) |
| **File references** | Relative paths from repo root |
| **Bold** | Key terms, component names, system names |
| **Line length** | No hard wrap requirement, but prefer ~100 chars for prose |
| **No secrets** | Never include real credentials, connection strings, or keys |

---

## Checklist for README Review

- [ ] Metadata table present with all required fields
- [ ] Table of Contents matches actual sections
- [ ] Overview describes service role and 4+ capabilities
- [ ] Architecture Mermaid diagram present
- [ ] Project structure tree present
- [ ] Domain capability map present
- [ ] Integration context documented
- [ ] Process flows documented for key runtime paths
- [ ] Interfaces/endpoints documented (if applicable)
- [ ] Data stores/state documented (if applicable)
- [ ] Configuration properties documented
- [ ] Dependencies table present
- [ ] Observability and security sections present
- [ ] Build/test commands present
- [ ] Deployment section present
- [ ] Change log present
- [ ] No secrets or real credentials
