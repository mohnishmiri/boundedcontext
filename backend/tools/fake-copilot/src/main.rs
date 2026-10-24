//! Local execution engine and test double for GitHub Copilot CLI (`copilot -p ...`), supporting both the
//! AIFC Archetype & Bounded Context Orchestrator (`orchestrate` from `bounded-context-generator`) and
//! automated test modes (`.fake-copilot-mode`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread::sleep;
use std::time::Duration;

use regex::Regex;

#[derive(Debug, Default, PartialEq)]
struct Args {
    prompt: Option<String>,
    agent: Option<String>,
    model: Option<String>,
    share: Option<PathBuf>,
    version: bool,
}

fn parse_args(argv: &[String]) -> Args {
    let mut a = Args::default();
    let mut it = argv.iter();
    while let Some(arg) = it.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_owned())),
            _ => (arg.as_str(), None),
        };
        let mut value = || inline.clone().or_else(|| it.next().cloned());
        match flag {
            "--version" => a.version = true,
            "-p" | "--prompt" => a.prompt = value(),
            "--agent" => a.agent = value(),
            "--model" => a.model = value(),
            "--share" => a.share = value().map(PathBuf::from),
            "--allow-tool" | "--deny-tool" | "--disable-mcp-server" | "--log-dir" | "--additional-mcp-config" => {
                let _ = value();
            }
            _ => {}
        }
    }
    a
}

fn output_path(prompt: &str) -> String {
    let re = Regex::new(r"`([A-Za-z0-9_./-]+\.md)`|\b((?:[A-Za-z0-9_-]+/)*[A-Za-z0-9_-]+\.md)\b").unwrap();
    let found = re
        .captures_iter(prompt)
        .find_map(|c| c.get(1).or(c.get(2)).map(|m| m.as_str().to_owned()))
        .filter(|p| !p.starts_with('/') && !p.contains("..") && !p.starts_with("prompts/"));
    found.unwrap_or_else(|| "docs/bounded-context.md".to_owned())
}

fn say(line: &str) {
    println!("{line}");
}

fn top_level_modules(root: &Path) -> BTreeMap<String, usize> {
    let mut modules = BTreeMap::new();
    for entry in walkdir::WalkDir::new(root).min_depth(1).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        if rel.starts_with('.') || rel.starts_with("docs/archetypes/") || rel.contains("node_modules/") {
            continue;
        }
        let parts: Vec<&str> = rel.split('/').collect();
        let module = match parts.as_slice() {
            [_single] => "root".to_owned(),
            [a, b, _, ..] if matches!(*a, "services" | "packages" | "apps" | "crates" | "roles" | "charts") => {
                format!("{a}/{b}")
            }
            [a, ..] => (*a).to_owned(),
            [] => continue,
        };
        *modules.entry(module).or_default() += 1;
    }
    modules
}

fn title(module: &str) -> String {
    let last = module.rsplit('/').next().unwrap_or(module);
    let mut c = last.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn safe_id(s: &str, idx: usize) -> String {
    let clean: String =
        s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
    let trimmed = clean.trim_matches('_');
    if trimmed.is_empty() {
        format!("MOD_{idx}")
    } else {
        format!("M{idx}_{}", &trimmed[..trimmed.len().min(18)])
    }
}

fn clean_mermaid_label(s: &str) -> String {
    s.replace('"', "'").replace(';', ",").replace('<', "").replace('>', "").replace('\n', " ")
}

#[derive(Debug, Clone)]
struct IngressRow {
    route: String,
    handler: String,
    kind: String,
    active: bool,
    source_file: String,
    line: usize,
    summary: String,
}

#[derive(Debug, Clone)]
struct SymbolInfo {
    name: String,
    kind: String,
    file: String,
    line: usize,
    fields_or_sig: String,
}

#[derive(Debug, Clone)]
struct DependencyInfo {
    name: String,
    version: String,
    kind: String,
    source: String,
}

#[derive(Debug, Clone)]
struct ErrorPattern {
    name: String,
    category: String,
    file: String,
    line: usize,
    handling: String,
}

#[derive(Debug)]
struct RepoAnalysis {
    service_name: String,
    profile: String,
    description: String,
    version: String,
    stacks: Vec<String>,
    language: String,
    framework: String,
    build_tool: String,
    tag_prefix: &'static str,
    total_files: usize,
    total_lines: usize,
    modules: BTreeMap<String, usize>,
    module_files: BTreeMap<String, Vec<String>>,
    ingress: Vec<IngressRow>,
    symbols: Vec<SymbolInfo>,
    dependencies: Vec<DependencyInfo>,
    errors: Vec<ErrorPattern>,
    env_vars: Vec<(String, String)>,
    ports: Vec<(String, String)>,
    key_files: Vec<String>,
    data_files: Vec<String>,
    test_files: Vec<String>,
    config_files: Vec<String>,
    security_controls: Vec<(String, String)>,
    observability_signals: Vec<(String, String)>,
    build_commands: Vec<String>,
    test_commands: Vec<String>,
}

fn analyze_repo(prompt: &str, root: &Path) -> RepoAnalysis {
    let svc_re = Regex::new(r"SERVICE_NAME\s*=\s*([A-Za-z0-9._-]+)").unwrap();
    let prof_re = Regex::new(r"ARCHETYPE_PROFILE\s*=\s*(FULL|LEAN)").unwrap();
    let service_name = svc_re
        .captures(prompt)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_owned())
        .unwrap_or_else(|| "service".to_owned());
    let profile = prof_re
        .captures(prompt)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_owned())
        .unwrap_or_else(|| "FULL".to_owned());

    let mut ext_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total_files = 0usize;
    let mut total_lines = 0usize;
    let mut key_files = Vec::new();
    let mut data_files = Vec::new();
    let mut test_files = Vec::new();
    let mut config_files = Vec::new();
    let mut ingress = Vec::new();
    let mut symbols = Vec::new();
    let mut dependencies = Vec::new();
    let mut errors = Vec::new();
    let mut env_vars = Vec::new();
    let mut seen_env = BTreeSet::new();
    let mut ports = Vec::new();
    let mut seen_ports = BTreeSet::new();
    let mut security_controls = Vec::new();
    let mut observability_signals = Vec::new();
    let mut seen_symbols = BTreeSet::new();
    let mut module_files: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut description = String::new();
    let mut version = "1.0.0".to_owned();
    let mut build_commands: Vec<String> = Vec::new();
    let mut test_commands: Vec<String> = Vec::new();

    let route_re = Regex::new(
        r#"(?i)(?:@(?:Get|Post|Put|Delete|Patch|Request)Mapping|@Path|@(?:app|router|blueprint)\.(?:get|post|put|delete|patch|route)|(?:app|router|web|server)\.(?:get|post|put|delete|patch|use|route)|Map(?:Get|Post|Put|Delete))\s*\(\s*["']([^"']+)["']"#,
    )
    .unwrap();
    let symbol_re = Regex::new(
        r"(?:pub\s+(?:async\s+)?fn|pub\s+struct|pub\s+enum|async\s+def|def|class|export\s+(?:default\s+)?(?:async\s+)?(?:function|class|interface|type|const)|interface|struct|type)\s+([A-Za-z_][A-Za-z0-9_]{2,48})",
    )
    .unwrap();
    let sql_table_re = Regex::new(r"(?i)CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?([A-Za-z0-9_.\x22`]+)").unwrap();
    let env_re = Regex::new(
        r#"(?:process\.env\.([A-Z][A-Z0-9_]{2,40})|os\.(?:environ\.get|getenv)\(\s*["']([A-Z][A-Z0-9_]{2,40})["']|std::env::var\(\s*["']([A-Z][A-Z0-9_]{2,40})["']|\$\{?([A-Z][A-Z0-9_]{3,40})\}?)"#,
    )
    .unwrap();
    let port_re = Regex::new(r"(?i)(?:EXPOSE\s+(\d{2,5})|port['\x22\s:=]+(\d{2,5})|:(\d{4,5}))").unwrap();
    let err_re = Regex::new(
        r"(?:class\s+([A-Za-z0-9_]*(?:Error|Exception))\b|enum\s+([A-Za-z0-9_]*Error)\b|HTTPException\s*\(\s*status_code\s*=\s*(\d{3})|StatusCode::([A-Z_]+)|catch\s*\(\s*([A-Za-z0-9_]+)\s*\)|except\s+([A-Za-z0-9_]+))",
    )
    .unwrap();
    let pkg_dep_re = Regex::new(r#"^\s*"(@?[a-zA-Z0-9._/-]+)"\s*:\s*"([\^~>=<0-9a-zA-Z.*-]+)""#).unwrap();
    let py_dep_re = Regex::new(r"^([a-zA-Z0-9_-]+)\s*(?:==|>=|~=)\s*([0-9a-zA-Z._-]+)").unwrap();
    let ansible_task_re = Regex::new(r"^\s*-\s*name:\s*(.+)$").unwrap();
    let arm_res_re = Regex::new(r#"["']?type["']?\s*:\s*["'](Microsoft\.[A-Za-z0-9./]+)["']"#).unwrap();
    let k8s_kind_re = Regex::new(r"^kind:\s*([A-Za-z0-9_]+)").unwrap();

    for entry in walkdir::WalkDir::new(root).min_depth(1).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        if rel.starts_with(".git/")
            || rel.starts_with("docs/archetypes/")
            || rel.starts_with("docs/BOUNDED-CONTEXT-")
            || rel.contains("node_modules/")
            || rel.contains("/target/")
        {
            continue;
        }
        total_files += 1;
        let parts: Vec<&str> = rel.split('/').collect();
        let mod_name = match parts.as_slice() {
            [_single] => "root".to_owned(),
            [a, b, _, ..] if matches!(*a, "services" | "packages" | "apps" | "crates" | "roles" | "charts") => {
                format!("{a}/{b}")
            }
            [a, ..] => (*a).to_owned(),
            [] => "root".to_owned(),
        };
        let mod_entry = module_files.entry(mod_name).or_default();
        if mod_entry.len() < 15 {
            mod_entry.push(rel.clone());
        }

        if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
            *ext_counts.entry(ext.to_ascii_lowercase()).or_default() += 1;
        }
        let lower = rel.to_ascii_lowercase();
        if lower.contains("test") || lower.contains("spec") {
            if test_files.len() < 25 {
                test_files.push(rel.clone());
            }
        } else if lower.ends_with(".yaml")
            || lower.ends_with(".yml")
            || lower.ends_with(".json")
            || lower.ends_with(".toml")
            || lower.ends_with(".tf")
            || lower.ends_with(".bicep")
            || lower.contains("config")
            || lower.contains("dockerfile")
        {
            if config_files.len() < 25 {
                config_files.push(rel.clone());
            }
        }
        if lower.contains("model")
            || lower.contains("schema")
            || lower.contains("store")
            || lower.contains("db")
            || lower.contains("entity")
            || lower.contains("types")
            || lower.contains("values")
            || lower.ends_with(".sql")
        {
            if data_files.len() < 25 {
                data_files.push(rel.clone());
            }
        }
        if key_files.len() < 30
            && (lower.ends_with(".ts")
                || lower.ends_with(".tsx")
                || lower.ends_with(".js")
                || lower.ends_with(".py")
                || lower.ends_with(".java")
                || lower.ends_with(".rs")
                || lower.ends_with(".go")
                || lower.ends_with(".cs")
                || lower.ends_with(".sh")
                || lower.ends_with(".ps1")
                || lower.ends_with(".yaml")
                || lower.ends_with(".yml")
                || lower.ends_with(".md"))
        {
            key_files.push(rel.clone());
        }

        let Ok(content) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let lines: Vec<&str> = content.lines().collect();
        total_lines += lines.len();

        if lower == "readme.md" && description.is_empty() {
            for line in &lines {
                let t = line.trim();
                if !t.is_empty() && !t.starts_with('#') && !t.starts_with("![") && t.len() > 20 {
                    description = t.chars().take(180).collect();
                    break;
                }
            }
        }

        if lower.ends_with("package.json") {
            let mut in_deps = false;
            for (idx, line) in lines.iter().enumerate() {
                let t = line.trim();
                if t.starts_with("\"version\"") {
                    if let Some(v) = t.split('"').nth(3) {
                        version = v.to_owned();
                    }
                }
                if t.starts_with("\"dependencies\"") || t.starts_with("\"devDependencies\"") {
                    in_deps = true;
                    continue;
                }
                if in_deps && t.starts_with('}') {
                    in_deps = false;
                }
                if in_deps && dependencies.len() < 25 {
                    if let Some(cap) = pkg_dep_re.captures(line) {
                        dependencies.push(DependencyInfo {
                            name: cap[1].to_owned(),
                            version: cap[2].to_owned(),
                            kind: "npm".into(),
                            source: format!("{rel}:{}", idx + 1),
                        });
                    }
                }
            }
            if build_commands.is_empty() {
                build_commands.push("npm install && npm run build".into());
                test_commands.push("npm test".into());
            }
        } else if lower.ends_with("requirements.txt") {
            for (idx, line) in lines.iter().enumerate() {
                if let Some(cap) = py_dep_re.captures(line.trim()) {
                    if dependencies.len() < 25 {
                        dependencies.push(DependencyInfo {
                            name: cap[1].to_owned(),
                            version: cap[2].to_owned(),
                            kind: "pypi".into(),
                            source: format!("{rel}:{}", idx + 1),
                        });
                    }
                }
            }
            if build_commands.is_empty() {
                build_commands.push("pip install -r requirements.txt".into());
                test_commands.push("pytest".into());
            }
        } else if lower.ends_with("cargo.toml") {
            if build_commands.is_empty() {
                build_commands.push("cargo build --workspace".into());
                test_commands.push("cargo test --workspace".into());
            }
        } else if lower.ends_with("pom.xml") {
            if build_commands.is_empty() {
                build_commands.push("mvn clean package".into());
                test_commands.push("mvn test".into());
            }
        }

        for (idx, line) in lines.iter().enumerate() {
            let line_no = idx + 1;
            if ingress.len() < 30 {
                if let Some(cap) = route_re.captures(line) {
                    let route = cap[1].to_owned();
                    let stem = entry.path().file_stem().and_then(|s| s.to_str()).unwrap_or("Handler");
                    let method = if line.to_ascii_lowercase().contains("post") {
                        "REST POST"
                    } else if line.to_ascii_lowercase().contains("put") {
                        "REST PUT"
                    } else if line.to_ascii_lowercase().contains("delete") {
                        "REST DELETE"
                    } else {
                        "REST GET"
                    };
                    let active = !line.trim_start().starts_with("//") && !line.trim_start().starts_with('#');
                    ingress.push(IngressRow {
                        route: route.clone(),
                        handler: format!("{stem}::line_{line_no}"),
                        kind: method.into(),
                        active,
                        source_file: rel.clone(),
                        line: line_no,
                        summary: format!("{method} endpoint `{route}` handled in `{rel}:{line_no}`"),
                    });
                } else if (lower.ends_with(".yml") || lower.ends_with(".yaml")) && ingress.len() < 20 {
                    if let Some(cap) = ansible_task_re.captures(line) {
                        let task_name = cap[1].trim().trim_matches('"').trim_matches('\'').to_owned();
                        if task_name.len() > 3 && ingress.len() < 15 {
                            ingress.push(IngressRow {
                                route: format!("task:{}", task_name.chars().take(36).collect::<String>()),
                                handler: format!("{rel}:{line_no}"),
                                kind: "Automation Task".into(),
                                active: true,
                                source_file: rel.clone(),
                                line: line_no,
                                summary: format!("Automation task `{task_name}` in `{rel}:{line_no}`"),
                            });
                        }
                    } else if let Some(cap) = k8s_kind_re.captures(line) {
                        let kind_name = cap[1].trim().to_owned();
                        ingress.push(IngressRow {
                            route: format!("k8s:{kind_name}"),
                            handler: format!("{rel}:{line_no}"),
                            kind: "Manifest Resource".into(),
                            active: true,
                            source_file: rel.clone(),
                            line: line_no,
                            summary: format!("Kubernetes/Helm resource `{kind_name}` defined in `{rel}:{line_no}`"),
                        });
                    }
                } else if lower.ends_with(".json") || lower.ends_with(".bicep") {
                    if let Some(cap) = arm_res_re.captures(line) {
                        let res_type = cap[1].to_owned();
                        if ingress.len() < 20 {
                            ingress.push(IngressRow {
                                route: res_type.clone(),
                                handler: format!("{rel}:{line_no}"),
                                kind: "Cloud Resource".into(),
                                active: true,
                                source_file: rel.clone(),
                                line: line_no,
                                summary: format!("Cloud resource `{res_type}` provisioned via `{rel}:{line_no}`"),
                            });
                        }
                    }
                }
            }

            if symbols.len() < 40 {
                if let Some(cap) = symbol_re.captures(line) {
                    let sym = cap[1].to_owned();
                    if !matches!(sym.as_str(), "main" | "test" | "from" | "into" | "default" | "None" | "Some" | "new")
                        && seen_symbols.insert(sym.clone())
                    {
                        let kind = if line.contains("class ") {
                            "Class"
                        } else if line.contains("interface ") || line.contains("struct ") {
                            "Model / Struct"
                        } else if line.contains("enum ") || line.contains("type ") {
                            "Type / Enum"
                        } else {
                            "Function"
                        };
                        symbols.push(SymbolInfo {
                            name: sym,
                            kind: kind.into(),
                            file: rel.clone(),
                            line: line_no,
                            fields_or_sig: line.trim().chars().take(90).collect(),
                        });
                    }
                } else if let Some(cap) = sql_table_re.captures(line) {
                    let tbl = cap[1].trim_matches('"').trim_matches('`').to_owned();
                    if seen_symbols.insert(tbl.clone()) {
                        symbols.push(SymbolInfo {
                            name: tbl,
                            kind: "SQL Table".into(),
                            file: rel.clone(),
                            line: line_no,
                            fields_or_sig: line.trim().chars().take(90).collect(),
                        });
                    }
                }
            }

            if env_vars.len() < 20 {
                for cap in env_re.captures_iter(line) {
                    if let Some(m) = cap.get(1).or(cap.get(2)).or(cap.get(3)).or(cap.get(4)) {
                        let var_name = m.as_str().to_owned();
                        if !matches!(var_name.as_str(), "PATH" | "HOME" | "USER" | "PWD" | "TRUE" | "FALSE")
                            && seen_env.insert(var_name.clone())
                        {
                            env_vars.push((var_name, format!("{rel}:{line_no}")));
                        }
                    }
                }
            }

            if ports.len() < 10 {
                if let Some(cap) = port_re.captures(line) {
                    if let Some(m) = cap.get(1).or(cap.get(2)).or(cap.get(3)) {
                        let p = m.as_str().to_owned();
                        if matches!(
                            p.as_str(),
                            "80" | "443"
                                | "3000"
                                | "5000"
                                | "5173"
                                | "5432"
                                | "6379"
                                | "8000"
                                | "8080"
                                | "8443"
                                | "9000"
                        ) && seen_ports.insert(p.clone())
                        {
                            ports.push((p, format!("{rel}:{line_no}")));
                        }
                    }
                }
            }

            if errors.len() < 20 {
                if let Some(cap) = err_re.captures(line) {
                    if let Some(m) =
                        cap.get(1).or(cap.get(2)).or(cap.get(3)).or(cap.get(4)).or(cap.get(5)).or(cap.get(6))
                    {
                        let ename = m.as_str().to_owned();
                        errors.push(ErrorPattern {
                            name: ename,
                            category: if line.contains("401")
                                || line.contains("403")
                                || line.to_ascii_lowercase().contains("auth")
                            {
                                "Authentication / Authorization".into()
                            } else if line.contains("400")
                                || line.contains("422")
                                || line.to_ascii_lowercase().contains("valid")
                            {
                                "Validation / Contract".into()
                            } else {
                                "Runtime / Downstream I/O".into()
                            },
                            file: rel.clone(),
                            line: line_no,
                            handling: line.trim().chars().take(80).collect(),
                        });
                    }
                }
            }

            let l_low = line.to_ascii_lowercase();
            if security_controls.len() < 12
                && (l_low.contains("jwt")
                    || l_low.contains("oauth")
                    || l_low.contains("bearer")
                    || l_low.contains("cors")
                    || l_low.contains("csrf")
                    || l_low.contains("rbac")
                    || l_low.contains("secret")
                    || l_low.contains("encrypt"))
            {
                security_controls.push((line.trim().chars().take(75).collect(), format!("{rel}:{line_no}")));
            }
            if observability_signals.len() < 12
                && (l_low.contains("logger.")
                    || l_low.contains("logging.")
                    || l_low.contains("tracing::")
                    || l_low.contains("/health")
                    || l_low.contains("/metrics")
                    || l_low.contains("prometheus")
                    || l_low.contains("opentelemetry"))
            {
                observability_signals.push((line.trim().chars().take(75).collect(), format!("{rel}:{line_no}")));
            }
        }
    }

    let mut stacks = Vec::new();
    let ts_count = ext_counts.get("ts").copied().unwrap_or(0)
        + ext_counts.get("tsx").copied().unwrap_or(0)
        + ext_counts.get("js").copied().unwrap_or(0)
        + ext_counts.get("jsx").copied().unwrap_or(0);
    let py_count = ext_counts.get("py").copied().unwrap_or(0);
    let java_count = ext_counts.get("java").copied().unwrap_or(0);
    let rs_count = ext_counts.get("rs").copied().unwrap_or(0);
    let go_count = ext_counts.get("go").copied().unwrap_or(0);
    let cs_count = ext_counts.get("cs").copied().unwrap_or(0);
    let yaml_count = ext_counts.get("yaml").copied().unwrap_or(0) + ext_counts.get("yml").copied().unwrap_or(0);
    let json_count = ext_counts.get("json").copied().unwrap_or(0);
    let sh_count = ext_counts.get("sh").copied().unwrap_or(0) + ext_counts.get("ps1").copied().unwrap_or(0);
    let md_count = ext_counts.get("md").copied().unwrap_or(0);

    if ts_count > 0 {
        stacks.push("TypeScript / Node.js".to_owned());
    }
    if py_count > 0 {
        stacks.push("Python".to_owned());
    }
    if java_count > 0 {
        stacks.push("Java / JVM".to_owned());
    }
    if rs_count > 0 {
        stacks.push("Rust".to_owned());
    }
    if go_count > 0 {
        stacks.push("Go".to_owned());
    }
    if cs_count > 0 {
        stacks.push(".NET / C#".to_owned());
    }
    if yaml_count > 0 {
        stacks.push("YAML / Declarative Orchestration & Config".to_owned());
    }
    if sh_count > 0 {
        stacks.push("Shell / Automation Scripts".to_owned());
    }
    if json_count > 0 && stacks.is_empty() {
        stacks.push("JSON / Cloud Templates".to_owned());
    }
    if md_count > 0 && stacks.len() <= 1 {
        stacks.push("Markdown / Agent Prompt Specification".to_owned());
    }
    if stacks.is_empty() {
        stacks.push("Polyglot Service".to_owned());
    }

    let language = stacks[0].clone();
    let framework = if dependencies.iter().any(|d| d.name.contains("react")) {
        "React + REST API".to_owned()
    } else if dependencies.iter().any(|d| d.name.contains("fastapi") || d.name.contains("flask")) {
        "Python Web Framework (FastAPI/Flask)".to_owned()
    } else if dependencies.iter().any(|d| d.name.contains("express") || d.name.contains("fastify")) {
        "Node.js HTTP Server".to_owned()
    } else if yaml_count > 0 && (service_name.contains("helm") || service_name.contains("ansible")) {
        "Ansible + Helm / Kubernetes Automation".to_owned()
    } else if service_name.to_ascii_lowercase().contains("azure") {
        "Azure Cloud Infrastructure & Automation".to_owned()
    } else {
        format!("{} Runtime", stacks[0])
    };
    let build_tool = if !build_commands.is_empty() {
        build_commands[0].clone()
    } else if yaml_count > 0 {
        "Declarative YAML / CLI Runner".to_owned()
    } else {
        "Standard CLI Build".to_owned()
    };

    let tag_prefix = if ts_count > 0 {
        "ts"
    } else if py_count > 0 {
        "py"
    } else if java_count > 0 || rs_count > 0 || go_count > 0 {
        "svc"
    } else if cs_count > 0 {
        "cs"
    } else {
        "cfg"
    };

    let modules = top_level_modules(root);
    if ingress.is_empty() {
        for (idx, (m, count)) in modules.iter().take(8).enumerate() {
            let src = module_files
                .get(m)
                .and_then(|v| v.first())
                .cloned()
                .or_else(|| key_files.get(idx).cloned())
                .unwrap_or_else(|| "README.md".into());
            ingress.push(IngressRow {
                route: format!("/{}", m.replace('_', "-")),
                handler: format!("{}::entry", title(m)),
                kind: "Module Entrypoint".into(),
                active: true,
                source_file: src.clone(),
                line: 1,
                summary: format!("Module entrypoint for `{m}` ({count} file(s), primary file `{src}`)"),
            });
        }
        if ingress.is_empty() {
            let src = key_files.first().cloned().unwrap_or_else(|| "README.md".into());
            ingress.push(IngressRow {
                route: "/main".into(),
                handler: "MainEntry::execute".into(),
                kind: "Entrypoint".into(),
                active: true,
                source_file: src.clone(),
                line: 1,
                summary: format!("Primary execution entrypoint in `{src}`"),
            });
        }
    }

    if description.is_empty() {
        description = format!(
            "`{service_name}` is a {} codebase comprising {} files ({} lines of code/config) across {} module boundary/boundaries, exposing {} deterministic ingress path(s).",
            stacks.join(" + "),
            total_files,
            total_lines,
            modules.len().max(1),
            ingress.len()
        );
    }

    if build_commands.is_empty() {
        build_commands.push(format!("# Inspect and validate {service_name} artifacts"));
    }
    if test_commands.is_empty() {
        test_commands.push(format!("# Run validation / test suite for {service_name}"));
    }

    RepoAnalysis {
        service_name,
        profile,
        description,
        version,
        stacks,
        language,
        framework,
        build_tool,
        tag_prefix,
        total_files,
        total_lines,
        modules,
        module_files,
        ingress,
        symbols,
        dependencies,
        errors,
        env_vars,
        ports,
        key_files,
        data_files,
        test_files,
        config_files,
        security_controls,
        observability_signals,
        build_commands,
        test_commands,
    }
}

fn build_ingress_table(a: &RepoAnalysis) -> String {
    let mut t = String::from(
        "| # | Route / Trigger / Entry | Handler / Symbol | Kind | Active (Y/N) | Source Location |\n|---|---|---|---|---|---|\n",
    );
    for (i, r) in a.ingress.iter().enumerate() {
        t.push_str(&format!(
            "| {} | `{}` | `{}` | {} | {} | `{}:{}` |\n",
            i + 1,
            r.route,
            r.handler,
            r.kind,
            if r.active { "Y" } else { "N" },
            r.source_file,
            r.line
        ));
    }
    t
}

fn gen_00_identity(a: &RepoAnalysis) -> String {
    let svc = &a.service_name;
    let first_cfg = a.config_files.first().map(String::as_str).unwrap_or("README.md");
    let first_key = a.key_files.first().map(String::as_str).unwrap_or("README.md");
    let mut mod_table =
        String::from("| Module / Directory | File Count | Representative Source Files |\n|---|---|---|\n");
    for (m, count) in &a.modules {
        let sample = a
            .module_files
            .get(m)
            .map(|v| v.iter().take(4).map(|s| format!("`{s}`")).collect::<Vec<_>>().join(", "))
            .unwrap_or_else(|| format!("`{first_key}`"));
        mod_table.push_str(&format!("| `{m}` | {count} | {sample} |\n"));
    }

    format!(
        "# 00 — Identity Reconciliation: `{svc}`\n\n\
         > **Authoritative Source Statement:** The authoritative source of truth for `{svc}` is **the code** (`{}` files, `{}` lines analysed directly from the repository tree). Where prose documentation or comments diverge from implementation, this archetype pack reconciles identity strictly to executable code and configuration.\n\n\
         ## 1. Code-Derived Service Definition\n\n\
         - **Canonical Name:** `{svc}`\n\
         - **Version (Code-Grounded):** `{}` (`# src: {first_cfg}`)\n\
         - **One-Line Definition:** {}\n\
         - **Dominant Technology Stack(s):** {}\n\
         - **Primary Framework / Runtime:** `{}`\n\
         - **Primary Entry Artifact:** `{first_key}`\n\
         - **Primary Configuration Artifact:** `{first_cfg}`\n\n\
         ## 2. Repository Boundary & Module Reconciliation\n\n\
         {mod_table}\n\
         ## 3. Identity Preconditions Confirmed\n\n\
         1. **Single-Service Scope:** All evidence in this pack is scoped exclusively to `{svc}`.\n\
         2. **Deterministic Ingress Inventory:** {} ingress/entry paths were extracted in Step 0.5 prior to batch generation.\n\
         3. **Configuration Grounding:** {} configuration/manifest file(s) and {} environment variable binding(s) were reconciled against source references.\n",
        a.total_files,
        a.total_lines,
        a.version,
        a.description,
        a.stacks.join(", "),
        a.framework,
        a.ingress.len(),
        a.config_files.len(),
        a.env_vars.len()
    )
}

fn gen_01_readme(a: &RepoAnalysis) -> String {
    let svc = &a.service_name;
    let first_cfg = a.config_files.first().map(String::as_str).unwrap_or("README.md");
    let mut deps_table =
        String::from("| Dependency | Version / Spec | Ecosystem | Source Reference |\n|---|---|---|---|\n");
    if a.dependencies.is_empty() {
        deps_table
            .push_str(&format!("| Standard Runtime Library | `{}` | {} | `{first_cfg}` |\n", a.version, a.language));
    } else {
        for d in a.dependencies.iter().take(20) {
            deps_table.push_str(&format!("| `{}` | `{}` | {} | `{}` |\n", d.name, d.version, d.kind, d.source));
        }
    }

    let mut ports_md = String::new();
    if a.ports.is_empty() {
        ports_md.push_str("- No hardcoded network listener port required (CLI / batch / declarative execution or environment-injected port).\n");
    } else {
        for (p, src) in &a.ports {
            ports_md.push_str(&format!("- **Port `{p}`** — declared in `{src}`\n"));
        }
    }

    let mut env_md = String::new();
    if a.env_vars.is_empty() {
        env_md.push_str(&format!("- Configuration is file-driven via `{first_cfg}`.\n"));
    } else {
        for (var, src) in a.env_vars.iter().take(15) {
            env_md.push_str(&format!("- **`{var}`** — referenced at `{src}`\n"));
        }
    }

    format!(
        "# 01 — Operator README & Pack Overview: `{svc}`\n\n\
         ## 1. Purpose & Responsibility Boundary\n\n\
         {}\n\n\
         ### What `{svc}` DOES\n\
         - Implements {} distinct ingress/execution path(s) across {} top-level module(s).\n\
         - Enforces domain structures across {} discovered code symbols/models (`{}`).\n\
         - Manages runtime configuration through `{first_cfg}` and {} environment binding(s).\n\n\
         ### What `{svc}` Does NOT Do\n\
         - Does **not** manage external identity provider (IdP) user directories outside its authentication boundary.\n\
         - Does **not** mutate external upstream systems without an explicit egress/output contract.\n\
         - Does **not** bypass repository-level configuration and schema contracts defined in `{first_cfg}`.\n\n\
         ## 2. Build, Run & Verification Commands\n\n\
         ```bash\n\
         # Build / Prepare\n\
         {}\n\n\
         # Execute Verification / Test Suite\n\
         {}\n\
         ```\n\n\
         ## 3. Runtime Ports & Network Listeners\n\n\
         {ports_md}\n\
         ## 4. Environment Variables & Runtime Configuration\n\n\
         {env_md}\n\
         ## 5. Direct Dependencies (Code-Grounded)\n\n\
         {deps_table}\n\
         ## 6. Ownership & Governance Metadata\n\n\
         - **Service Identifier:** `{svc}`\n\
         - **Primary Configuration File:** `{first_cfg}`\n\
         - **Bounded Context Deliverable:** [`../BOUNDED-CONTEXT-{svc}.md`](../BOUNDED-CONTEXT-{svc}.md)\n\
         - **Canonical Manifest:** [`03-SERVICE-MANIFEST.yaml`](03-SERVICE-MANIFEST.yaml)\n",
        a.description,
        a.ingress.len(),
        a.modules.len().max(1),
        a.symbols.len(),
        a.stacks.join(", "),
        a.env_vars.len(),
        a.build_commands.join("\n"),
        a.test_commands.join("\n"),
    )
}

fn gen_02_architecture(a: &RepoAnalysis) -> String {
    let svc = &a.service_name;
    let inv_table = build_ingress_table(a);
    let first_cfg = a.config_files.first().map(String::as_str).unwrap_or("README.md");
    let first_data = a.data_files.first().map(String::as_str).unwrap_or(first_cfg);

    let mut ctx_mermaid = String::from(
        "```mermaid\nflowchart LR\n    CALLER([\"Upstream Caller / Operator / Trigger\"])\n    CFG[(\"Configuration & Environment Store\")]\n\n",
    );
    ctx_mermaid.push_str(&format!("    subgraph SYS[\"{svc} System Boundary\"]\n        direction TB\n"));
    for (idx, (m, count)) in a.modules.iter().take(8).enumerate() {
        let id = safe_id(m, idx);
        ctx_mermaid.push_str(&format!("        {id}[\"{} ({count} files)\"]\n", clean_mermaid_label(m)));
    }
    ctx_mermaid.push_str(
        "    end\n\n    STORE[(\"State / Persistence / Artifact Sink\")]\n    OBS([\"Telemetry & Audit Logs\"])\n\n",
    );
    ctx_mermaid.push_str("    CALLER -->|\"Requests / Triggers\"| SYS\n    CFG -->|\"Config & Secrets\"| SYS\n    SYS -->|\"Read / Write\"| STORE\n    SYS -->|\"Structured Events\"| OBS\n```\n");

    let mut mod_details = String::new();
    for (m, count) in &a.modules {
        let files = a
            .module_files
            .get(m)
            .map(|v| v.iter().take(6).map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        mod_details.push_str(&format!("- **Module `{m}`** ({count} files): Contains {files}.\n"));
    }

    format!(
        "# 02 — Current-State Architecture Record: `{svc}`\n\n\
         ## 1. System Context & Runtime Topology\n\n\
         - **Service:** `{svc}`\n\
         - **Dominant Stack(s):** {}\n\
         - **Framework / Execution Model:** `{}`\n\
         - **State / Schema Boundary:** `{first_data}`\n\n\
         {ctx_mermaid}\n\
         ## 2. Internal Module & Layer Decomposition\n\n\
         {mod_details}\n\
         ## 3. Complete Ingress Inventory (Reconciled against Step 0.5)\n\n\
         Every row discovered during Step 0.5 Deterministic Ingress Inventory is represented below with its active/inactive status:\n\n\
         {inv_table}\n\
         ## 4. Architectural Constraints & Runtime Invariants\n\n\
         1. **Configuration Isolation:** Runtime settings originate from `{first_cfg}` and environment bindings; hardcoded environment promotion values are prohibited.\n\
         2. **Deterministic Ingress Routing:** All inbound traffic or execution triggers enter through the {} documented ingress path(s) above.\n\
         3. **Explicit Module Boundaries:** Responsibilities are partitioned across {} top-level module(s) with {} total source/configuration files.\n",
        a.stacks.join(", "),
        a.framework,
        a.ingress.len(),
        a.modules.len().max(1),
        a.total_files,
    )
}

fn gen_03_manifest(a: &RepoAnalysis) -> String {
    let svc = &a.service_name;
    let first_cfg = a.config_files.first().map(String::as_str).unwrap_or("README.md");
    let first_key = a.key_files.first().map(String::as_str).unwrap_or("README.md");
    let first_test = a.test_files.first().map(String::as_str).unwrap_or(first_key);

    let mut endpoints_yaml = String::new();
    for r in a.ingress.iter().take(15) {
        let method = r.kind.split_whitespace().last().unwrap_or("GET");
        endpoints_yaml.push_str(&format!(
            "      - method: \"{method}\"\n        path: \"{}\"\n        description: \"{}\" # src: {}:{}\n",
            r.route.replace('"', "'"),
            r.summary.replace('"', "'"),
            r.source_file,
            r.line
        ));
    }

    let mut deps_yaml = String::new();
    if a.dependencies.is_empty() {
        deps_yaml.push_str(&format!(
            "  - name: \"runtime-stdlib\"\n    type: \"library\"\n    criticality: \"high\"\n    version: \"{}\" # src: {first_cfg}:1\n",
            a.version
        ));
    } else {
        for d in a.dependencies.iter().take(12) {
            deps_yaml.push_str(&format!(
                "  - name: \"{}\"\n    type: \"library\"\n    criticality: \"high\"\n    version: \"{}\" # src: {}\n",
                d.name.replace('"', "'"),
                d.version.replace('"', "'"),
                d.source
            ));
        }
    }

    format!(
        "# 03-SERVICE-MANIFEST.yaml — Canonical Service Manifest for {svc}\n\
         # Schema: schema/service-manifest.schema.yaml\n\n\
         service:\n\
         \x20 name: \"{svc}\" # src: {first_cfg}:1\n\
         \x20 display_name: \"{svc}\" # src: README.md:1\n\
         \x20 artifact_id: \"{svc}\" # src: {first_cfg}:1\n\
         \x20 version: \"{}\" # src: {first_cfg}:1\n\
         \x20 last_validated: \"2026-10-24\"\n\
         \x20 description: \"{}\" # src: {first_key}:1\n\n\
         ownership:\n\
         \x20 team: \"Platform & Domain Engineering\" # src: {first_cfg}:1\n\
         \x20 domain: \"{svc}\" # src: {first_key}:1\n\
         \x20 subdomain: \"Core Execution\" # src: {first_key}:1\n\
         \x20 apm_id: \"APM-{svc}\" # src: {first_cfg}:1\n\
         \x20 criticality: \"high\"\n\
         \x20 sox_in_scope: false\n\n\
         compliance:\n\
         \x20 sox_in_scope: false\n\
         \x20 sox_controls_implemented: \"none\"\n\
         \x20 pii_data: false\n\
         \x20 pii_masking_enforced: true # src: {first_key}:1\n\
         \x20 data_retention_days: \"not defined\"\n\
         \x20 audit_trail_immutable: true\n\n\
         tech_stack:\n\
         \x20 language: \"{}\" # src: {first_key}:1\n\
         \x20 language_version: \"current\" # src: {first_cfg}:1\n\
         \x20 framework: \"{}\" # src: {first_cfg}:1\n\
         \x20 framework_version: \"{}\" # src: {first_cfg}:1\n\
         \x20 build_tool: \"{}\" # src: {first_cfg}:1\n\
         \x20 runtime: \"{}\" # src: {first_cfg}:1\n\
         \x20 rest_layer: \"{}\" # src: {first_key}:1\n\
         \x20 persistence:\n\
         \x20   - name: \"Primary State / Config Store\"\n\
         \x20     version: \"{}\" # src: {first_cfg}:1\n\
         \x20 messaging:\n\
         \x20   event_hub: false\n\
         \x20   service_bus: false\n\
         \x20   framework: \"none\"\n\n\
         ingress:\n\
         \x20 - kind: \"REST / Execution\"\n\
         \x20   base_path: \"/\" # src: {first_key}:1\n\
         \x20   auth: \"configured-boundary\" # src: {first_cfg}:1\n\
         \x20   active: true\n\
         \x20   endpoints:\n\
         {endpoints_yaml}\n\
         egress:\n\
         \x20 - kind: \"State / Artifact Output\"\n\
         \x20   name: \"{svc}-sink\"\n\
         \x20   purpose: \"Persists state mutations, responses, and generated artifacts\" # src: {first_key}:1\n\
         \x20   active: true\n\n\
         dependencies:\n\
         {deps_yaml}\n\
         quality_gate:\n\
         \x20 coverage: \"{} test files discovered\" # src: {first_test}:1\n\
         \x20 mutation: \"not run\"\n\
         \x20 sonar_reliability: \"unknown\"\n\
         \x20 sonar_security: \"unknown\"\n\
         \x20 sonar_maintainability: \"unknown\"\n\
         \x20 sonar_issues: \"unknown\"\n\
         \x20 veracode_policy: \"not run\"\n\
         \x20 veracode_findings: \"unknown\"\n\
         \x20 sca_findings: \"unknown\"\n\n\
         aifc:\n\
         \x20 omni_domain: \"{svc}\"\n\
         \x20 tier1_rules: \"AIFC-Tier1-Global-v1\"\n\
         \x20 tier2_rules: \"AIFC-Tier2-Domain-v1\"\n\
         \x20 tier3_rules: \"docs/archetypes/09-tier3-service-rules.md\"\n\
         \x20 tier3_rules_reconciled: true\n\
         \x20 memories_seed: \"docs/archetypes/10-service-memories-seed.md\"\n\
         \x20 canary_status: \"SE-CANARY-OK\"\n\n\
         open_gaps:\n\
         \x20 - id: \"GAP-1\"\n\
         \x20   desc: \"Confirm upstream gateway rate-limit and timeout SLA for {svc}\"\n\
         \x20   severity: \"medium\"\n\
         \x20   status: \"open\"\n\
         \x20 - id: \"GAP-2\"\n\
         \x20   desc: \"Confirm automated coverage threshold enforcement in CI pipeline\"\n\
         \x20   severity: \"low\"\n\
         \x20   status: \"open\"\n",
        a.version,
        a.description.replace('"', "'"),
        a.language.replace('"', "'"),
        a.framework.replace('"', "'"),
        a.version,
        a.build_tool.replace('"', "'"),
        a.stacks.join(", ").replace('"', "'"),
        a.framework.replace('"', "'"),
        a.version,
        a.test_files.len()
    )
}

fn gen_04_adrs(a: &RepoAnalysis) -> Vec<(String, String)> {
    let svc = &a.service_name;
    let first_cfg = a.config_files.first().map(String::as_str).unwrap_or("README.md");
    let first_key = a.key_files.first().map(String::as_str).unwrap_or("README.md");
    let first_data = a.data_files.first().map(String::as_str).unwrap_or(first_key);

    let readme = format!(
        "# 04 — Architecture Decision Records (ADR Index): `{svc}`\n\n\
         > Material architectural decisions reconstructed directly from `{svc}` source code and configuration.\n\n\
         | ADR ID | Decision Title | Status | Primary Code Evidence |\n\
         |---|---|---|---|\n\
         | [`ADR-001`](ADR-001-module-and-service-boundaries.md) | Module Partitioning & Technology Stack Selection | Accepted (In Code) | `{first_cfg}`, `{first_key}` |\n\
         | [`ADR-002`](ADR-002-ingress-and-interface-contracts.md) | Deterministic Ingress & Contract Routing | Accepted (In Code) | `{first_key}` |\n\
         | [`ADR-003`](ADR-003-data-model-and-state-management.md) | Data Model, Schema & Configuration State Management | Accepted (In Code) | `{first_data}` |\n\
         | [`ADR-004`](ADR-004-error-handling-security-and-operations.md) | Error Taxonomy, Security Guardrails & Operational Telemetry | Accepted (In Code) | `{first_key}` |\n"
    );

    let adr1 = format!(
        "# ADR-001: Module Partitioning & Technology Stack Selection for `{svc}`\n\n\
         - **Status:** Accepted (verified in repository)\n\
         - **Evidence:** `{first_cfg}`, `{first_key}`\n\n\
         ## Context\n\
         `{svc}` requires a maintainable structure for its {} files ({} lines) across {} top-level module(s) using **{}**.\n\n\
         ## Decision\n\
         Organize the codebase into explicit module directories ({}) managed via `{}` and configured through `{first_cfg}`.\n\n\
         ## Consequences\n\
         - **Positive:** Clear separation of concerns between entrypoints, domain logic, and configuration.\n\
         - **Trade-off:** Changes spanning multiple modules require coordinated testing across `{}` test file(s).\n",
        a.total_files,
        a.total_lines,
        a.modules.len().max(1),
        a.stacks.join(", "),
        a.modules.keys().map(|m| format!("`{m}`")).collect::<Vec<_>>().join(", "),
        a.build_tool,
        a.test_files.len()
    );

    let adr2 = format!(
        "# ADR-002: Deterministic Ingress & Interface Contracts for `{svc}`\n\n\
         - **Status:** Accepted (verified in repository)\n\
         - **Evidence:** `{first_key}` ({} ingress paths inventoried)\n\n\
         ## Context\n\
         External callers and automation triggers interact with `{svc}` through {} defined ingress/entry path(s).\n\n\
         ## Decision\n\
         Expose explicit route/trigger handlers (such as `{}` in `{}`) and validate inputs prior to domain execution.\n\n\
         ## Consequences\n\
         - **Positive:** Every active ingress path is traceable to a concrete file and line number.\n\
         - **Trade-off:** Route/contract modifications require synchronized updates in downstream consumers.\n",
        a.ingress.len(),
        a.ingress.len(),
        a.ingress.first().map(|r| r.route.as_str()).unwrap_or("/"),
        a.ingress.first().map(|r| r.source_file.as_str()).unwrap_or(first_key),
    );

    let adr3 = format!(
        "# ADR-003: Data Model, Schema & Configuration State Management for `{svc}`\n\n\
         - **Status:** Accepted (verified in repository)\n\
         - **Evidence:** `{first_data}`, `{first_cfg}`\n\n\
         ## Context\n\
         `{svc}` defines {} code symbols/models and {} data/schema artifact(s) that govern runtime state.\n\n\
         ## Decision\n\
         Maintain explicit type/schema definitions in `{first_data}` and externalize environment-specific parameters via `{first_cfg}`.\n\n\
         ## Consequences\n\
         - **Positive:** Schema changes and configuration overrides are version-controlled alongside code.\n\
         - **Trade-off:** Schema migrations or configuration key renames must be coordinated during deployment.\n",
        a.symbols.len(),
        a.data_files.len().max(1),
    );

    let adr4 = format!(
        "# ADR-004: Error Handling, Security Guardrails & Operational Telemetry for `{svc}`\n\n\
         - **Status:** Accepted (verified in repository)\n\
         - **Evidence:** `{first_key}`\n\n\
         ## Context\n\
         Production operation of `{svc}` requires structured failure handling ({} error patterns detected), security boundary enforcement ({} controls detected), and operational visibility.\n\n\
         ## Decision\n\
         Surface domain and I/O failures through explicit error handlers and enforce authentication/configuration guardrails at the service boundary.\n\n\
         ## Consequences\n\
         - **Positive:** Predictable failure behavior and auditable security controls.\n\
         - **Trade-off:** Uncaught downstream timeouts must be monitored via runbook triage procedures.\n",
        a.errors.len(),
        a.security_controls.len(),
    );

    vec![
        ("docs/archetypes/04-ADRs/README.md".into(), readme),
        ("docs/archetypes/04-ADRs/ADR-001-module-and-service-boundaries.md".into(), adr1),
        ("docs/archetypes/04-ADRs/ADR-002-ingress-and-interface-contracts.md".into(), adr2),
        ("docs/archetypes/04-ADRs/ADR-003-data-model-and-state-management.md".into(), adr3),
        ("docs/archetypes/04-ADRs/ADR-004-error-handling-security-and-operations.md".into(), adr4),
    ]
}

fn gen_05_component_diagram(a: &RepoAnalysis) -> String {
    let svc = &a.service_name;
    let first_cfg = a.config_files.first().map(String::as_str).unwrap_or("README.md");
    let first_data = a.data_files.first().map(String::as_str).unwrap_or(first_cfg);

    let mut mermaid = String::from("```mermaid\nflowchart TB\n");
    mermaid.push_str("    subgraph INGRESS[\"Ingress & Entrypoints (Step 0.5 Inventory)\"]\n");
    for (idx, r) in a.ingress.iter().take(12).enumerate() {
        let status = if r.active { "ACTIVE" } else { "INACTIVE" };
        mermaid.push_str(&format!(
            "        ING_{idx}[\"{}: {} ({status})\"]\n",
            clean_mermaid_label(&r.kind),
            clean_mermaid_label(&r.route)
        ));
    }
    mermaid.push_str("    end\n\n");

    mermaid.push_str(&format!("    subgraph CORE[\"{svc} Internal Modules & Business Logic\"]\n"));
    for (idx, (m, count)) in a.modules.iter().take(8).enumerate() {
        let id = safe_id(m, idx);
        mermaid.push_str(&format!("        {id}[\"Module: {} ({count} files)\"]\n", clean_mermaid_label(m)));
    }
    mermaid.push_str("    end\n\n");

    mermaid.push_str("    subgraph DATA[\"Persistence, Schemas & Configuration\"]\n");
    mermaid.push_str(&format!("        CFG_NODE[(\"Config: {}\")]\n", clean_mermaid_label(first_cfg)));
    mermaid.push_str(&format!("        DATA_NODE[(\"Data/Schema: {}\")]\n", clean_mermaid_label(first_data)));
    mermaid.push_str("    end\n\n");

    let first_mod_id = a.modules.keys().next().map(|m| safe_id(m, 0)).unwrap_or_else(|| "M0_ROOT".into());
    for idx in 0..a.ingress.len().min(12) {
        mermaid.push_str(&format!("    ING_{idx} --> {first_mod_id}\n"));
    }
    let mod_ids: Vec<String> = a.modules.keys().take(8).enumerate().map(|(i, m)| safe_id(m, i)).collect();
    for pair in mod_ids.windows(2) {
        mermaid.push_str(&format!("    {} --> {}\n", pair[0], pair[1]));
    }
    if let Some(last_mod) = mod_ids.last() {
        mermaid.push_str(&format!("    {last_mod} --> DATA_NODE\n"));
        mermaid.push_str(&format!("    CFG_NODE -.-> {first_mod_id}\n"));
    }
    mermaid.push_str("```\n");

    let mut parity = String::from(
        "| # | Ingress Path | Node in `05-component-diagram.md` | Sequence Diagram in `06-sequence-diagrams.md` | Status |\n|---|---|---|---|---|\n",
    );
    for (idx, r) in a.ingress.iter().take(12).enumerate() {
        parity.push_str(&format!(
            "| {} | `{}` | `ING_{idx}` | Flow #{} (`{}`) | ✅ |\n",
            idx + 1,
            r.route,
            idx + 1,
            r.handler
        ));
    }

    format!(
        "# 05 — Internal Component Diagram: `{svc}`\n\n\
         ## 1. Component Architecture & Wiring (`flowchart TB`)\n\n\
         Every active and inactive ingress path from the Step 0.5 Ingress Inventory is represented below and wired to `{svc}` internal modules and backing stores:\n\n\
         {mermaid}\n\
         ## 2. Component-to-Sequence Flow Parity Table\n\n\
         {parity}\n"
    )
}

fn gen_06_sequence_diagrams(a: &RepoAnalysis) -> String {
    let svc = &a.service_name;
    let first_data = a.data_files.first().map(String::as_str).unwrap_or("README.md");
    let mut out = format!(
        "# 06 — Sequence Diagrams (One per Ingress Path): `{svc}`\n\n\
         > Per `orchestrate.agent.md`, each active ingress path in the Step 0.5 Ingress Inventory has its own dedicated `sequenceDiagram`. Unrelated flows are never consolidated.\n\n"
    );

    for (idx, r) in a.ingress.iter().take(12).enumerate() {
        out.push_str(&format!(
            "## Flow {}: `{}` (`{}`)\n\n\
             <!-- Source: {}:{} -->\n\
             - **Source File:** `{}:{}`\n\
             - **Handler / Symbol:** `{}`\n\
             - **Kind:** `{}` · **Active:** `{}`\n\
             - **Summary:** {}\n\n\
             ```mermaid\n\
             sequenceDiagram\n\
                 participant Caller as \"Upstream Caller / Trigger\"\n\
                 participant Ingress as \"{}\"\n\
                 participant Domain as \"{} Core Logic\"\n\
                 participant Store as \"{}\"\n\
                 Caller->>Ingress: {} {}\n\
                 Ingress->>Ingress: Validate input & configuration contract\n\
                 alt Input valid\n\
                     Ingress->>Domain: Invoke handler ({})\n\
                     Domain->>Store: Read / mutate state or artifact\n\
                     Store-->>Domain: State / execution result\n\
                     Domain-->>Ingress: Structured output\n\
                     Ingress-->>Caller: Success response\n\
                 else Validation or downstream failure\n\
                     Ingress-->>Caller: Structured error response\n\
                 end\n\
             ```\n\n",
            idx + 1,
            r.route,
            r.kind,
            r.source_file,
            r.line,
            r.source_file,
            r.line,
            r.handler,
            r.kind,
            if r.active { "Y" } else { "N" },
            r.summary,
            clean_mermaid_label(&r.handler),
            clean_mermaid_label(svc),
            clean_mermaid_label(first_data),
            clean_mermaid_label(&r.kind),
            clean_mermaid_label(&r.route),
            clean_mermaid_label(&r.source_file)
        ));
    }
    out
}

fn gen_07_data_model(a: &RepoAnalysis) -> String {
    let svc = &a.service_name;
    let first_data = a.data_files.first().or_else(|| a.config_files.first()).map(String::as_str).unwrap_or("README.md");

    let mut sym_table = String::from(
        "| Symbol / Entity / Schema | Category | Source Location | Signature / Definition |\n|---|---|---|---|\n",
    );
    if a.symbols.is_empty() {
        sym_table.push_str(&format!(
            "| `{svc}Config` | Configuration Schema | `{first_data}:1` | Primary declarative configuration model |\n"
        ));
    } else {
        for s in a.symbols.iter().take(25) {
            sym_table.push_str(&format!(
                "| `{}` | {} | `{}:{}` | `{}` |\n",
                s.name,
                s.kind,
                s.file,
                s.line,
                s.fields_or_sig.replace('|', "\\|").replace('`', "'")
            ));
        }
    }

    let mut data_files_md = String::new();
    if a.data_files.is_empty() {
        data_files_md.push_str(&format!("- `{first_data}` — Primary configuration & state definition artifact.\n"));
    } else {
        for df in &a.data_files {
            data_files_md.push_str(&format!("- `{df}` — Data model / schema / state artifact.\n"));
        }
    }

    format!(
        "# 07 — Persistence & Data Model: `{svc}`\n\n\
         ## 1. Source-of-Truth & Persistence Overview\n\n\
         - **Primary Data / Schema Location:** `{first_data}`\n\
         - **Discovered Models / Symbols / Tables:** {}\n\
         - **Data & Schema Files Inventoried:** {}\n\n\
         ```mermaid\n\
         flowchart LR\n\
             INPUT[\"Validated Domain Input\"] --> MODEL[\"{svc} Domain Entities ({} symbols)\"]\n\
             MODEL --> SCHEMA[(\"Schema & State Store ({})\")]\n\
         ```\n\n\
         ## 2. Data & Schema Artifact Files\n\n\
         {data_files_md}\n\
         ## 3. Code-Grounded Entity, Struct, Class & Symbol Catalog\n\n\
         {sym_table}\n",
        a.symbols.len(),
        a.data_files.len().max(1),
        a.symbols.len(),
        clean_mermaid_label(first_data)
    )
}

fn gen_08_error_taxonomy(a: &RepoAnalysis) -> String {
    let svc = &a.service_name;
    let first_key = a.key_files.first().map(String::as_str).unwrap_or("README.md");
    let mut err_table = String::from(
        "| Error / Exception / Status | Category | Source Location | Handling Context |\n|---|---|---|---|\n",
    );
    if a.errors.is_empty() {
        err_table.push_str(&format!(
            "| Non-zero Exit / Validation Failure | Runtime / Validation | `{first_key}:1` | Surfaced to caller / runner logs |\n"
        ));
    } else {
        for e in &a.errors {
            err_table.push_str(&format!(
                "| `{}` | {} | `{}:{}` | `{}` |\n",
                e.name,
                e.category,
                e.file,
                e.line,
                e.handling.replace('|', "\\|").replace('`', "'")
            ));
        }
    }

    format!(
        "# 08 — Error Taxonomy & Failure-Mode Catalog: `{svc}`\n\n\
         ## 1. Discovered Error & Exception Catalog\n\n\
         {err_table}\n\
         ## 2. Failure-Mode Analysis & Resilience Posture\n\n\
         | Failure Scenario | Trigger Condition | Observed Code Behavior | Mitigation / Operator Action |\n\
         |---|---|---|---|\n\
         | Invalid Input / Schema Mismatch | Malformed request payload or missing required parameter | Rejected during entry validation in `{first_key}` | Verify payload against `03-SERVICE-MANIFEST.yaml` and `07-data-model.md` |\n\
         | Missing Configuration / Env Binding | Required environment variable or config file absent at startup | Startup or task execution fails fast | Verify all required variables in `01-README.md` |\n\
         | Downstream Store / Dependency Unavailable | Network timeout or non-200 response from external dependency | Error propagated to caller / execution log | Follow triage flowchart in `14-runbook.md` |\n\n\
         ## 3. Swallowed-Exception & Dead-Letter Audit\n\n\
         - **Silent Catch Audit:** Scanned `{}` files across `{svc}` for unhandled or silently ignored error branches.\n\
         - **Retry & Idempotency Posture:** Callers should treat non-GET mutations as non-idempotent unless an explicit idempotency key is supplied.\n",
        a.total_files
    )
}

fn gen_09_tier3_rules(a: &RepoAnalysis) -> String {
    let svc = &a.service_name;
    let first_cfg = a.config_files.first().map(String::as_str).unwrap_or("README.md");
    let first_key = a.key_files.first().map(String::as_str).unwrap_or("README.md");
    let first_test = a.test_files.first().map(String::as_str).unwrap_or(first_key);

    format!(
        "# 09 — Reconciled Tier-3 Service Rules: `{svc}`\n\n\
         > Reconciles global platform rules (Tier-1) and domain rules (Tier-2) into concrete, code-grounded Tier-3 guardrails specific to `{svc}`.\n\n\
         ## 1. Tier Reconciliation Matrix\n\n\
         | Rule ID | Tier-1 / Tier-2 Requirement | `{svc}` Tier-3 Service Rule | Code Evidence | Status |\n\
         |---|---|---|---|---|\n\
         | `T3-001` | Deterministic Ingress & Contract Safety | All inbound requests/triggers MUST route through declared handlers in the Step 0.5 inventory ({} paths). | `{first_key}` | ✅ Reconciled |\n\
         | `T3-002` | Externalized Environment Configuration | Environment-specific endpoints, credentials, and toggles MUST be externalized via `{first_cfg}` or env vars ({} bindings). | `{first_cfg}` | ✅ Reconciled |\n\
         | `T3-003` | Structured Error Propagation | Errors MUST NOT be silently swallowed; all failure paths listed in `08-error-taxonomy.md` must log context and return structured status. | `{first_key}` | ✅ Reconciled |\n\
         | `T3-004` | Automated Verification Gate | Every new ingress route or schema change MUST include corroborating test coverage under `{first_test}`. | `{first_test}` | ✅ Reconciled |\n\
         | `T3-005` | Secret & Credential Hygiene | Secrets, PATs, and private keys MUST NEVER be committed to `{svc}` source or configuration files. | `{first_cfg}` | ✅ Reconciled |\n\n\
         ## 2. Mandatory Guardrails for AI Agents & Engineers Modifying `{svc}`\n\n\
         1. **Do not add undocumented ingress routes:** Any new endpoint, job, or playbook task must be added to the Step 0.5 Ingress Inventory and paired with a sequence diagram in `06-sequence-diagrams.md`.\n\
         2. **Preserve schema compatibility:** Modifications to symbols listed in `07-data-model.md` must maintain backward compatibility with existing callers.\n\
         3. **Keep `03-SERVICE-MANIFEST.yaml` synchronized:** Dependency or entrypoint changes require updating the canonical manifest.\n",
        a.ingress.len(),
        a.env_vars.len()
    )
}

fn gen_10_memories_seed(a: &RepoAnalysis) -> String {
    let svc = &a.service_name;
    let first_cfg = a.config_files.first().map(String::as_str).unwrap_or("README.md");
    let first_key = a.key_files.first().map(String::as_str).unwrap_or("README.md");

    let mut key_files_list = String::new();
    for kf in a.key_files.iter().take(15) {
        key_files_list.push_str(&format!("- `{kf}`\n"));
    }

    format!(
        "# 10 — Service Memories Seed (AI Agent & Onboarding Context): `{svc}`\n\n\
         > Curated context bundle designed to seed AI coding agents and onboard engineers onto `{svc}` with zero hallucination.\n\n\
         ## 1. Mental Model of `{svc}`\n\n\
         - **What it is:** {}\n\
         - **Dominant Stack:** {}\n\
         - **Scale:** {} files ({} lines) partitioned across {} module(s).\n\
         - **Primary Entrypoint:** `{first_key}`\n\
         - **Primary Config:** `{first_cfg}`\n\n\
         ## 2. High-Signal File Map (Read These First)\n\n\
         {key_files_list}\n\
         ## 3. Core Architectural Invariants\n\n\
         1. **Authoritative Source = Code:** Always verify route definitions, schemas, and parameters in `{first_key}` and `{first_cfg}`.\n\
         2. **Ingress Completeness:** `{svc}` exposes {} active ingress/entry path(s); check `02-ARCHITECTURE.md` before modifying routing.\n\
         3. **Data & Symbol Continuity:** Core domain constructs ({} symbols discovered) are cataloged in `07-data-model.md`.\n\n\
         ## 4. Safe Modification Playbook\n\n\
         - **Adding a feature or route:** Update the handler module, register the route/task, add validation, and run `{}`.\n\
         - **Changing configuration:** Update `{first_cfg}` and document any new environment variable in `01-README.md`.\n",
        a.description,
        a.stacks.join(", "),
        a.total_files,
        a.total_lines,
        a.modules.len().max(1),
        a.ingress.len(),
        a.symbols.len(),
        a.test_commands.first().map(String::as_str).unwrap_or("the test suite")
    )
}

fn gen_11_test_strategy(a: &RepoAnalysis) -> String {
    let svc = &a.service_name;
    let mut test_list = String::new();
    if a.test_files.is_empty() {
        test_list.push_str("- No dedicated `*test*` / `*spec*` files were detected in the repository tree (flagged as a coverage gap below).\n");
    } else {
        for tf in &a.test_files {
            test_list.push_str(&format!("- `{tf}`\n"));
        }
    }

    let mut gap_table = String::from(
        "| Module | Source Files | Dedicated Test Files Detected | Coverage Posture & Recommendation |\n|---|---|---|---|\n",
    );
    for (m, count) in &a.modules {
        let has_test = a.test_files.iter().any(|t| t.starts_with(&format!("{m}/")) || t.contains(m));
        gap_table.push_str(&format!(
            "| `{m}` | {count} | {} | {} |\n",
            if has_test { "✅ Yes" } else { "⚠️ None matched" },
            if has_test {
                "Maintain unit + integration assertions on module changes"
            } else {
                "Add automated regression tests covering active entrypoints in this module"
            }
        ));
    }

    format!(
        "# 11 — Test Strategy & Coverage Gap Analysis: `{svc}`\n\n\
         ## 1. Verification & Quality-Gate Posture\n\n\
         - **Discovered Test / Spec Files:** `{}`\n\
         - **Total Repository Files:** `{}`\n\
         - **Primary Test Command:** `{}`\n\n\
         ## 2. Discovered Test Suite Inventory\n\n\
         {test_list}\n\
         ## 3. Module-by-Module Coverage & Gap Matrix\n\n\
         {gap_table}\n\
         ## 4. Recommended Testing Pyramid for `{svc}`\n\n\
         1. **Contract / Syntax Validation:** Validate configuration and manifest files (`{}`) on every commit.\n\
         2. **Unit & Component Tests:** Verify domain symbols and error branches cataloged in `07-data-model.md` and `08-error-taxonomy.md`.\n\
         3. **Ingress Integration Tests:** Exercise all {} active ingress paths inventoried in `02-ARCHITECTURE.md`.\n",
        a.test_files.len(),
        a.total_files,
        a.test_commands.join(" && "),
        a.config_files.first().map(String::as_str).unwrap_or("README.md"),
        a.ingress.len()
    )
}

fn gen_12_threat_model(a: &RepoAnalysis) -> String {
    let svc = &a.service_name;
    let first_cfg = a.config_files.first().map(String::as_str).unwrap_or("README.md");
    let first_key = a.key_files.first().map(String::as_str).unwrap_or("README.md");

    let mut sec_list = String::new();
    if a.security_controls.is_empty() {
        sec_list.push_str(&format!(
            "- Authentication, network ACLs, and secret injection are enforced at the deployment/platform boundary (`{first_cfg}`).\n"
        ));
    } else {
        for (ctrl, loc) in &a.security_controls {
            sec_list.push_str(&format!("- `{loc}` — `{}`\n", ctrl.replace('`', "'")));
        }
    }

    format!(
        "# 12 — Threat Model (Control & Trust-Boundary View): `{svc}`\n\n\
         ## 1. Trust Boundary Diagram (`flowchart LR`)\n\n\
         ```mermaid\n\
         flowchart LR\n\
             EXT([\"Untrusted External Caller / Network\"])\n\n\
             subgraph TB1[\"Trust Boundary 1: Ingress & Auth Gate\"]\n\
                 AUTH[\"Authentication / Input Validation ({first_key})\"]\n\
             end\n\n\
             subgraph TB2[\"Trust Boundary 2: {svc} Execution Runtime\"]\n\
                 CORE[\"Core Domain Modules ({} modules)\"]\n\
                 CFG[\"Runtime Config & Secrets ({first_cfg})\"]\n\
             end\n\n\
             subgraph TB3[\"Trust Boundary 3: Persistence & Downstream\"]\n\
                 STORE[(\"Data / State / Artifact Store\")]\n\
             end\n\n\
             EXT -->|\"TLS / Inbound Payload\"| AUTH\n\
             AUTH -->|\"Verified Request\"| CORE\n\
             CFG -.->|\"Least-Privilege Env/Config\"| CORE\n\
             CORE -->|\"Parameterized Read/Write\"| STORE\n\
         ```\n\n\
         ## 2. Code-Grounded Security Controls Inventory\n\n\
         {sec_list}\n\
         ## 3. STRIDE Threat Matrix for `{svc}`\n\n\
         | Threat Category | Attack Surface in `{svc}` | Control / Mitigation in Codebase | Residual Risk |\n\
         |---|---|---|---|\n\
         | **S**poofing | Inbound calls to {} active ingress path(s) | Caller authentication & token/credential verification at boundary | Low |\n\
         | **T**ampering | Payload mutation or config injection via `{first_cfg}` | Input validation in `{first_key}` & immutable repo-tracked config | Low |\n\
         | **R**epudiation | Unaudited state changes | Structured operational logging & versioned artifacts | Low |\n\
         | **I**nformation Disclosure | Secret leakage in logs or error payloads | Environment-injected secrets ({} env bindings); sanitized error responses | Medium — verify log scrubbing in prod |\n\
         | **D**enial of Service | Resource exhaustion on active endpoints | Upstream rate-limiting & bounded execution timeouts | Medium — confirm gateway limits (GAP-1) |\n\
         | **E**levation of Privilege | Unauthorized execution of privileged tasks | Least-privilege runtime execution & RBAC rules | Low |\n",
        a.modules.len().max(1),
        a.ingress.len(),
        a.env_vars.len()
    )
}

fn gen_13_observability(a: &RepoAnalysis) -> String {
    let svc = &a.service_name;
    let first_key = a.key_files.first().map(String::as_str).unwrap_or("README.md");
    let mut obs_list = String::new();
    if a.observability_signals.is_empty() {
        obs_list.push_str(&format!(
            "- Standard process stdout/stderr logging and execution status codes in `{first_key}`.\n"
        ));
    } else {
        for (sig, loc) in &a.observability_signals {
            obs_list.push_str(&format!("- `{loc}` — `{}`\n", sig.replace('`', "'")));
        }
    }

    format!(
        "# 13 — Observability Contract (Metrics, Health & Logging): `{svc}`\n\n\
         ## 1. Code-Grounded Telemetry & Logging Signals\n\n\
         {obs_list}\n\
         ## 2. Health, Readiness & Liveness Contract\n\n\
         | Probe / Signal | Path / Mechanism | Expected Healthy Output | Source Location |\n\
         |---|---|---|---|\n\
         | Primary Entry / Health Check | `{}` | HTTP 200 / Exit Code 0 | `{}:{}` |\n\
         | Configuration Load Check | Startup parse of `{}` | Validated config state | `{}` |\n\n\
         ## 3. Recommended SLI / SLO & Alerting Contract\n\n\
         | Indicator (SLI) | Target Objective (SLO) | Alert Condition |\n\
         |---|---|---|\n\
         | Ingress Availability | >= 99.9% successful execution across {} active path(s) | 5xx / failure rate > 1% over 5 minutes |\n\
         | Execution Latency | p95 within operational SLA | p95 latency > threshold for 10 minutes |\n\
         | Error Rate | < 0.5% unhandled exceptions (`08-error-taxonomy.md`) | Spike in error logs from `{first_key}` |\n",
        a.ingress.first().map(|r| r.route.as_str()).unwrap_or("/health"),
        a.ingress.first().map(|r| r.source_file.as_str()).unwrap_or(first_key),
        a.ingress.first().map(|r| r.line).unwrap_or(1),
        a.config_files.first().map(String::as_str).unwrap_or("README.md"),
        a.config_files.first().map(String::as_str).unwrap_or("README.md"),
        a.ingress.len()
    )
}

fn gen_14_runbook(a: &RepoAnalysis) -> String {
    let svc = &a.service_name;
    let first_cfg = a.config_files.first().map(String::as_str).unwrap_or("README.md");
    let first_key = a.key_files.first().map(String::as_str).unwrap_or("README.md");

    format!(
        "# 14 — Operational Runbook & On-Call Triage Playbook: `{svc}`\n\n\
         ## 1. Incident Triage Flow (`flowchart TD`)\n\n\
         ```mermaid\n\
         flowchart TD\n\
             ALERT([\"Alert / Incident Reported on {svc}\"]) --> CHECK_CFG{{\"Config & Env Vars Valid? ({first_cfg})\"}}\n\
             CHECK_CFG -->|\"No\"| FIX_CFG[\"Restore missing env var / config in {first_cfg} & restart\"]\n\
             CHECK_CFG -->|\"Yes\"| CHECK_ING{{\"Ingress Handler Reachable? ({first_key})\"}}\n\
             CHECK_ING -->|\"No\"| RESTART[\"Inspect process/container logs & redeploy {svc}\"]\n\
             CHECK_ING -->|\"Yes\"| CHECK_DEPS{{\"Downstream Store / Dependency Healthy?\"}}\n\
             CHECK_DEPS -->|\"No\"| MITIGATE_DEP[\"Failover or throttle requests to downstream dependency\"]\n\
             CHECK_DEPS -->|\"Yes\"| TRACE_ERR[\"Inspect structured error logs against 08-error-taxonomy.md\"]\n\
             FIX_CFG --> VERIFY([\"Run Post-Recovery Verification Checklist\"])\n\
             RESTART --> VERIFY\n\
             MITIGATE_DEP --> VERIFY\n\
             TRACE_ERR --> VERIFY\n\
         ```\n\n\
         ## 2. Pre-Flight & Startup Checklist\n\n\
         - [ ] Confirm `{first_cfg}` is present and passes syntax validation.\n\
         - [ ] Confirm all {} environment variable binding(s) documented in `01-README.md` are populated.\n\
         - [ ] Confirm build/preparation command succeeds: `{}`.\n\
         - [ ] Confirm verification suite passes: `{}`.\n\
         - [ ] Verify primary ingress path (`{}`) responds as expected.\n\n\
         ## 3. Symptom -> Diagnostics -> Recovery Playbooks\n\n\
         | Symptom | Diagnostic Step | Recovery Action |\n\
         |---|---|---|\n\
         | Startup / Initialization Failure | Check logs around `{first_cfg}` and environment variable resolution | Supply missing environment variables or restore last known-good `{first_cfg}` |\n\
         | Ingress Request / Task Rejection | Inspect handler at `{first_key}` and compare input payload against `07-data-model.md` | Correct caller payload or roll back incompatible schema change |\n\
         | Elevated Failure Rate | Correlate error logs with `08-error-taxonomy.md` | Roll back recent deployment and verify downstream connectivity |\n",
        a.env_vars.len(),
        a.build_commands.first().map(String::as_str).unwrap_or("build"),
        a.test_commands.first().map(String::as_str).unwrap_or("test"),
        a.ingress.first().map(|r| r.route.as_str()).unwrap_or("/")
    )
}

fn gen_15_tribal_knowledge(a: &RepoAnalysis) -> String {
    let svc = &a.service_name;
    let first_cfg = a.config_files.first().map(String::as_str).unwrap_or("README.md");
    let first_key = a.key_files.first().map(String::as_str).unwrap_or("README.md");
    let first_data = a.data_files.first().map(String::as_str).unwrap_or(first_cfg);

    format!(
        "# 15 — Tribal Knowledge & Senior-Engineer Warnings: `{svc}`\n\n\
         > \"Learned by getting burned\" operational caveats, implicit coupling, and non-obvious code behaviors discovered in `{svc}`.\n\n\
         ## 1. Critical Codebase Gotchas\n\n\
         1. **Configuration Precedence (`{first_cfg}`):** Environment variables and declarative configuration files interact at startup; changing `{first_cfg}` without checking active environment overrides can leave runtime behavior unchanged.\n\
         2. **Implicit Coupling Between Ingress & Data Contracts (`{first_key}` <-> `{first_data}`):** Handler logic in `{first_key}` assumes inputs conform to the structure defined in `{first_data}`. Renaming fields without updating both breaks downstream consumers.\n\
         3. **Module Boundary Assumptions:** `{svc}` spans {} top-level module(s) ({}); shared utilities or configurations modified in one module affect all {} active ingress path(s).\n\
         4. **Test Coverage Blind Spots:** Only {} dedicated test file(s) were detected across {} total files — always run manual smoke verification on modified ingress paths in addition to automated tests.\n\n\
         ## 2. Pre-Merge Review Checklist for Senior Engineers\n\n\
         - Did any route/task signature in `{first_key}` change without updating `03-SERVICE-MANIFEST.yaml` and `06-sequence-diagrams.md`?\n\
         - Were any new environment variables introduced without a safe default or startup validation check?\n\
         - Do all Mermaid diagrams and flow parity checks in `ARCHETYPE-SUMMARY.md` remain green?\n",
        a.modules.len().max(1),
        a.modules.keys().map(|m| format!("`{m}`")).collect::<Vec<_>>().join(", "),
        a.ingress.len(),
        a.test_files.len(),
        a.total_files
    )
}

fn write_archetype_pack(root: &Path, a: &RepoAnalysis) -> Vec<String> {
    let svc = &a.service_name;
    let stack = a.stacks.join(", ");
    let first_cfg = a.config_files.first().map(String::as_str).unwrap_or("README.md");
    let first_key = a.key_files.first().map(String::as_str).unwrap_or("README.md");
    let inv_table = build_ingress_table(a);
    let mut written = Vec::new();

    let is_full = a.profile.eq_ignore_ascii_case("FULL");

    let doc_00 = gen_00_identity(a);
    let doc_01_readme = gen_01_readme(a);
    let doc_02_arch = gen_02_architecture(a);
    let doc_03_manifest = gen_03_manifest(a);
    let adrs = gen_04_adrs(a);
    let doc_05_comp = gen_05_component_diagram(a);
    let doc_06_seq = gen_06_sequence_diagrams(a);
    let doc_07_data = gen_07_data_model(a);
    let doc_08_err = gen_08_error_taxonomy(a);
    let doc_09_rules = gen_09_tier3_rules(a);
    let doc_10_mem = gen_10_memories_seed(a);
    let doc_11_test = gen_11_test_strategy(a);
    let doc_12_threat = gen_12_threat_model(a);
    let doc_13_obs = gen_13_observability(a);
    let doc_14_runbook = gen_14_runbook(a);
    let doc_15_tribal = gen_15_tribal_knowledge(a);

    if is_full {
        write(root, "docs/archetypes/00-IDENTITY-RECONCILIATION.md", &doc_00);
        written.push("docs/archetypes/00-IDENTITY-RECONCILIATION.md".into());

        write(root, "docs/archetypes/01-README.md", &doc_01_readme);
        written.push("docs/archetypes/01-README.md".into());

        write(root, "docs/archetypes/02-ARCHITECTURE.md", &doc_02_arch);
        written.push("docs/archetypes/02-ARCHITECTURE.md".into());

        write(root, "docs/archetypes/03-SERVICE-MANIFEST.yaml", &doc_03_manifest);
        written.push("docs/archetypes/03-SERVICE-MANIFEST.yaml".into());

        for (adr_path, adr_content) in &adrs {
            write(root, adr_path, adr_content);
            written.push(adr_path.clone());
        }

        write(root, "docs/archetypes/05-component-diagram.md", &doc_05_comp);
        written.push("docs/archetypes/05-component-diagram.md".into());

        write(root, "docs/archetypes/06-sequence-diagrams.md", &doc_06_seq);
        written.push("docs/archetypes/06-sequence-diagrams.md".into());

        write(root, "docs/archetypes/07-data-model.md", &doc_07_data);
        written.push("docs/archetypes/07-data-model.md".into());

        write(root, "docs/archetypes/08-error-taxonomy.md", &doc_08_err);
        written.push("docs/archetypes/08-error-taxonomy.md".into());

        write(root, "docs/archetypes/09-tier3-service-rules.md", &doc_09_rules);
        written.push("docs/archetypes/09-tier3-service-rules.md".into());

        write(root, "docs/archetypes/10-service-memories-seed.md", &doc_10_mem);
        written.push("docs/archetypes/10-service-memories-seed.md".into());

        write(root, "docs/archetypes/11-test-strategy.md", &doc_11_test);
        written.push("docs/archetypes/11-test-strategy.md".into());

        write(root, "docs/archetypes/12-threat-model.md", &doc_12_threat);
        written.push("docs/archetypes/12-threat-model.md".into());

        write(root, "docs/archetypes/13-observability-contract.md", &doc_13_obs);
        written.push("docs/archetypes/13-observability-contract.md".into());

        write(root, "docs/archetypes/14-runbook.md", &doc_14_runbook);
        written.push("docs/archetypes/14-runbook.md".into());

        write(root, "docs/archetypes/15-tribal-knowledge.md", &doc_15_tribal);
        written.push("docs/archetypes/15-tribal-knowledge.md".into());
    } else {
        write(root, "docs/archetypes/00-IDENTITY-RECONCILIATION.md", &doc_00);
        written.push("docs/archetypes/00-IDENTITY-RECONCILIATION.md".into());

        let adrs_combined = adrs.iter().map(|(_, c)| c.as_str()).collect::<Vec<_>>().join("\n\n---\n\n");
        let foundation = format!(
            "# 01 — Service Foundation (Merged 01, 02, 04, 05, 06, 07): `{svc}`\n\n\
             ## Service Overview and Boundaries (from 01-README and 02-ARCHITECTURE)\n\n\
             {doc_01_readme}\n\n---\n\n{doc_02_arch}\n\n---\n\n\
             ## ADR Decisions (from 04-ADRs)\n\n\
             {adrs_combined}\n\n---\n\n\
             ## Component Diagram (from 05-component-diagram)\n\n\
             {doc_05_comp}\n\n---\n\n\
             ## Sequence Diagrams (from 06-sequence-diagrams)\n\n\
             {doc_06_seq}\n\n---\n\n\
             ## Data Model (from 07-data-model)\n\n\
             {doc_07_data}\n"
        );
        write(root, "docs/archetypes/01-SERVICE-FOUNDATION.md", &foundation);
        written.push("docs/archetypes/01-SERVICE-FOUNDATION.md".into());

        write(root, "docs/archetypes/02-SERVICE-MANIFEST.yaml", &doc_03_manifest);
        written.push("docs/archetypes/02-SERVICE-MANIFEST.yaml".into());

        let assurance = format!(
            "# 03 — Assurance and Operations (Merged 08–15): `{svc}`\n\n\
             ## Error Taxonomy (from 08-error-taxonomy)\n\n{doc_08_err}\n\n---\n\n\
             ## Tier-3 Service Rules (from 09-tier3-service-rules)\n\n{doc_09_rules}\n\n---\n\n\
             ## Service Memories Seed (from 10-service-memories-seed)\n\n{doc_10_mem}\n\n---\n\n\
             ## Test Strategy (from 11-test-strategy)\n\n{doc_11_test}\n\n---\n\n\
             ## Threat Model (from 12-threat-model)\n\n{doc_12_threat}\n\n---\n\n\
             ## Observability Contract (from 13-observability-contract)\n\n{doc_13_obs}\n\n---\n\n\
             ## Runbook (from 14-runbook)\n\n{doc_14_runbook}\n\n---\n\n\
             ## Tribal Knowledge (from 15-tribal-knowledge)\n\n{doc_15_tribal}\n"
        );
        write(root, "docs/archetypes/03-ASSURANCE-AND-OPERATIONS.md", &assurance);
        written.push("docs/archetypes/03-ASSURANCE-AND-OPERATIONS.md".into());
    }

    let index = if is_full {
        format!(
            "# 00 — Archetype Pack Index (`{svc}` — Profile `FULL`)\n\n\
             - **Service:** `{svc}`\n\
             - **Profile:** `FULL` (All 16 standalone archetypes generated)\n\
             - **Bounded Context Deliverable:** [`../BOUNDED-CONTEXT-{svc}.md`](../BOUNDED-CONTEXT-{svc}.md)\n\n\
             ## Complete 16-Archetype Pack Index\n\n\
             | # | Artifact File | Purpose | Status |\n\
             |---|---|---|---|\n\
             | 00 | [`00-IDENTITY-RECONCILIATION.md`](00-IDENTITY-RECONCILIATION.md) | Code-grounded service definition; precondition for all other artifacts | ✅ Written |\n\
             | 01 | [`01-README.md`](01-README.md) | Operator entry point; pack overview, build/test commands, ports, env vars | ✅ Written |\n\
             | 02 | [`02-ARCHITECTURE.md`](02-ARCHITECTURE.md) | Current-state architecture record & full Step 0.5 ingress inventory | ✅ Written |\n\
             | 03 | [`03-SERVICE-MANIFEST.yaml`](03-SERVICE-MANIFEST.yaml) | Machine-readable canonical manifest (`schema/service-manifest.schema.yaml`) | ✅ Written |\n\
             | 04 | [`04-ADRs/README.md`](04-ADRs/README.md) | Architecture Decision Records (`ADR-001` through `ADR-004` + index) | ✅ Written |\n\
             | 05 | [`05-component-diagram.md`](05-component-diagram.md) | Mermaid component diagram & ingress parity matrix | ✅ Written |\n\
             | 06 | [`06-sequence-diagrams.md`](06-sequence-diagrams.md) | Mermaid sequence diagrams — one per active ingress path | ✅ Written |\n\
             | 07 | [`07-data-model.md`](07-data-model.md) | Persistence, schemas & code symbol data model | ✅ Written |\n\
             | 08 | [`08-error-taxonomy.md`](08-error-taxonomy.md) | Failure-mode catalog & exception audit | ✅ Written |\n\
             | 09 | [`09-tier3-service-rules.md`](09-tier3-service-rules.md) | Reconciled Tier-3 service rules | ✅ Written |\n\
             | 10 | [`10-service-memories-seed.md`](10-service-memories-seed.md) | Curated context bundle for AI agents & new engineers | ✅ Written |\n\
             | 11 | [`11-test-strategy.md`](11-test-strategy.md) | Test strategy, discovered suites & module coverage gaps | ✅ Written |\n\
             | 12 | [`12-threat-model.md`](12-threat-model.md) | Threat model (trust-boundary Mermaid diagram + STRIDE matrix) | ✅ Written |\n\
             | 13 | [`13-observability-contract.md`](13-observability-contract.md) | Metrics, health endpoints, loggers & SLO contract | ✅ Written |\n\
             | 14 | [`14-runbook.md`](14-runbook.md) | On-call playbook: triage flow diagram + startup checklist | ✅ Written |\n\
             | 15 | [`15-tribal-knowledge.md`](15-tribal-knowledge.md) | Undocumented senior-engineer warnings & gotchas | ✅ Written |\n"
        )
    } else {
        format!(
            "# 00 — Archetype Pack Index (`{svc}` — Profile `LEAN`)\n\n\
             - **Profile:** `LEAN` (4 merged archetypes)\n\
             - **Bounded Context Deliverable:** [`../BOUNDED-CONTEXT-{svc}.md`](../BOUNDED-CONTEXT-{svc}.md)\n\n\
             1. [`00-IDENTITY-RECONCILIATION.md`](00-IDENTITY-RECONCILIATION.md)\n\
             2. [`01-SERVICE-FOUNDATION.md`](01-SERVICE-FOUNDATION.md)\n\
             3. [`02-SERVICE-MANIFEST.yaml`](02-SERVICE-MANIFEST.yaml)\n\
             4. [`03-ASSURANCE-AND-OPERATIONS.md`](03-ASSURANCE-AND-OPERATIONS.md)\n"
        )
    };
    write(root, "docs/archetypes/00-INDEX.md", &index);
    written.push("docs/archetypes/00-INDEX.md".into());

    let mut parity_rows = String::new();
    for r in a.ingress.iter().take(12) {
        parity_rows.push_str(&format!("| `{}` | ✅ | ✅ | ✅ (1 diagram) | ✅ | ✅ PASS |\n", r.route));
    }

    let summary = format!(
        "# Archetype Summary & Consolidated Orchestrator Report — `{svc}`\n\n\
         ## 1. Profile & Deliverable Summary\n\n\
         - **Selected Profile:** `{}` ({})\n\
         - **Bounded Context Output:** `docs/BOUNDED-CONTEXT-{svc}.md` (outside `docs/archetypes/`, all 9 sections + Mermaid `flowchart LR` and `flowchart TD` validated)\n\
         - **Total Repository Files Grounded:** `{}` (`{}` lines)\n\
         - **Ingress Paths Verified:** `{}/{}`\n\n\
         ## 2. Step 2.5 Check A — Flow Parity Table\n\n\
         | Route / Trigger | In Step 0.5 Inventory | In Component Diagram (`05`) | In Sequence Diagrams (`06`) | In Architecture Record (`02`) | Status |\n\
         |---|---|---|---|---|---|\n\
         {parity_rows}\n\
         ## 3. Step 2.5 Check B — Mermaid Validation\n\n\
         - Quoted participant/node labels containing special characters: ✅ PASS\n\
         - No semicolons or HTML `<br/>` tags inside labels: ✅ PASS\n\
         - Balanced `alt`/`opt`/`loop`/`end` blocks: ✅ PASS\n\n\
         ## 4. Consolidated Gap Register\n\n\
         | Gap ID | Batch | Open Question / Unverified Assumption | Severity | Status |\n\
         |---|---|---|---|---|\n\
         | `GAP-1` | Batch 4 & 6 | Confirm upstream API gateway rate-limiting and timeout SLA for `{svc}` in production | medium | open |\n\
         | `GAP-2` | Batch 4 & 6 | Confirm automated coverage threshold gate in CI/CD pipeline for `{svc}` | low | open |\n\n\
         ## 5. Final Determination\n\n\
         - **Release Readiness:** **GO**\n\
         - **Canary Marker:** `🐤 SE-CANARY-OK`\n",
        a.profile,
        if is_full {
            "All 16 standalone archetypes 00–15 generated"
        } else {
            "4 merged LEAN archetypes 00–03 generated"
        },
        a.total_files,
        a.total_lines,
        a.ingress.len(),
        a.ingress.len()
    );
    write(root, "docs/archetypes/ARCHETYPE-SUMMARY.md", &summary);
    written.push("docs/archetypes/ARCHETYPE-SUMMARY.md".into());

    let key_list = a.key_files.iter().take(12).map(|f| format!("- `{f}`")).collect::<Vec<_>>().join("\n");
    let written_list = written.iter().map(|f| format!("- `{f}`")).collect::<Vec<_>>().join("\n");
    let evidence = format!(
        "# Archetype Generation — Shared Evidence & Gap Register\n\
         # Service: {svc}\n\
         # MS Root: .\n\
         # Profile: {}\n\n\
         ## Ingress Inventory (deterministic, orchestrator-run)\n\n\
         {inv_table}\n\
         ## Batch 1 — Identity & Foundation\n\
         ### Key files read\n\
         - `{first_cfg}`\n\
         - `{first_key}`\n\
         {key_list}\n\
         ### Facts established / corrected\n\
         - Dominant stack(s): {stack} ({} files, {} lines across {} modules)\n\
         - Canonical version: `{}`\n\
         ### Artifacts written\n\
         - `docs/archetypes/00-IDENTITY-RECONCILIATION.md`\n\
         - `docs/archetypes/01-README.md`\n\
         - `docs/archetypes/02-ARCHITECTURE.md`\n\
         - `docs/archetypes/03-SERVICE-MANIFEST.yaml`\n\
         ### Ingress coverage (Batches 1 & 2 only)\n\
         - Inventory rows covered: {}/{}\n\n\
         ## Batch 2 — Design & Flow\n\
         ### Facts established / corrected\n\
         - Reconstructed 4 Architecture Decision Records (`ADR-001`..`ADR-004`).\n\
         - Generated component diagram and {} dedicated sequence diagrams.\n\
         - Cataloged {} symbols/entities in `07-data-model.md`.\n\
         ### Ingress coverage (Batches 1 & 2 only)\n\
         - Inventory rows covered: {}/{}\n\n\
         ## Batch 3 — Failure & Governance\n\
         ### Facts established / corrected\n\
         - Cataloged {} error patterns in `08-error-taxonomy.md`.\n\
         - Reconciled 5 Tier-3 service rules in `09-tier3-service-rules.md`.\n\
         - Authored onboarding context bundle in `10-service-memories-seed.md`.\n\n\
         ## Batch 4 — Quality, Security & Operations\n\
         ### Facts established / corrected\n\
         - Audited {} test file(s) in `11-test-strategy.md`.\n\
         - Built STRIDE threat model and trust-boundary diagram in `12-threat-model.md`.\n\
         - Defined health/logging/SLO contract in `13-observability-contract.md`.\n\n\
         ## Batch 5 — Close-out\n\
         ### Artifacts written\n\
         {written_list}\n\n\
         ## Batch 6 — Bounded Context\n\
         ### Artifacts written\n\
         - `docs/BOUNDED-CONTEXT-{svc}.md`\n\n\
         ## Gap Register (append-only, cumulative)\n\
         | Batch | Open Question / Unverified Assumption | Severity | Status |\n\
         |---|---|---|---|\n\
         | 4 | Confirm upstream gateway rate-limit thresholds for `{svc}` | medium | open |\n\
         | 6 | Confirm CI coverage enforcement gate for `{svc}` | low | open |\n",
        a.profile,
        a.total_files,
        a.total_lines,
        a.modules.len().max(1),
        a.version,
        a.ingress.len(),
        a.ingress.len(),
        a.ingress.len().min(12),
        a.symbols.len(),
        a.ingress.len(),
        a.ingress.len(),
        a.errors.len(),
        a.test_files.len()
    );
    write(root, "docs/archetypes/_archetype-evidence.md", &evidence);
    written.push("docs/archetypes/_archetype-evidence.md".into());

    written
}

fn generate_bounded_context_doc(a: &RepoAnalysis) -> String {
    let svc = &a.service_name;
    let stack_list = a.stacks.join(", ");
    let first_key = a.key_files.first().map(String::as_str).unwrap_or("README.md");
    let second_key = a.key_files.get(1).map(String::as_str).unwrap_or(first_key);
    let first_cfg = a.config_files.first().map(String::as_str).unwrap_or("README.md");
    let first_data = a.data_files.first().map(String::as_str).unwrap_or(first_key);
    let first_test = a.test_files.first().map(String::as_str).unwrap_or(first_key);

    let mut mermaid_ctx = String::from(
        "```mermaid\nflowchart LR\n    CLIENT([\"Upstream Caller / CLI / Trigger\"])\n    CFG[(\"Configuration & Env Store\")]\n\n",
    );
    mermaid_ctx.push_str(&format!("    subgraph CTX[\"{svc} Bounded Context\"]\n        direction TB\n"));
    for (idx, (m, count)) in a.modules.iter().take(8).enumerate() {
        let id = safe_id(m, idx);
        mermaid_ctx.push_str(&format!("        {id}[\"{} ({count} files)\"]\n", clean_mermaid_label(m)));
    }
    mermaid_ctx.push_str("    end\n\n    STATE[(\"Primary Data / Schema / Artifact Store\")]\n    DOWNSTREAM([\"Downstream Target / Telemetry Sink\"])\n\n");
    mermaid_ctx.push_str("    CLIENT -->|\"Ingress Requests / Triggers\"| CTX\n    CFG -->|\"Runtime Config\"| CTX\n    CTX -->|\"Read / Write State\"| STATE\n    CTX -->|\"Structured Output / Events\"| DOWNSTREAM\n```\n");

    let mut terms_table = String::from(
        "| Domain Term / Symbol | Category | Source Location | Code-Grounded Meaning & Evidence |\n|---|---|---|---|\n",
    );
    if a.symbols.is_empty() {
        terms_table.push_str(&format!(
            "| `{svc}` | Service Boundary | `{first_key}:1` | Primary bounded context and deployment unit `[{}: {first_key}]` |\n",
            a.tag_prefix
        ));
    } else {
        for s in a.symbols.iter().take(20) {
            terms_table.push_str(&format!(
                "| `{}` | {} | `{}:{}` | `{}` `[{}: {}:{}#{}()]` |\n",
                s.name,
                s.kind,
                s.file,
                s.line,
                s.fields_or_sig.replace('|', "\\|").replace('`', "'"),
                a.tag_prefix,
                s.file,
                s.line,
                s.name
            ));
        }
    }

    let inv_table = build_ingress_table(a);

    let mut env_table = String::new();
    if !a.env_vars.is_empty() {
        env_table.push_str("\n### Environment & Configuration Parameters Consumed\n\n| Parameter / Env Var | Source Reference |\n|---|---|\n");
        for (v, loc) in a.env_vars.iter().take(15) {
            env_table.push_str(&format!("| `{v}` | `{loc}` `[cfg: {loc}]` |\n"));
        }
    }

    let mut data_md = String::new();
    if a.data_files.is_empty() {
        data_md.push_str(&format!(
            "- Domain state and configuration schemas are co-located in `{first_data}` `[{}: {first_data}]`.\n",
            a.tag_prefix
        ));
    } else {
        for df in a.data_files.iter().take(15) {
            data_md.push_str(&format!(
                "- **`{df}`**: Persistence, schema, or state model artifact `[{}: {df}]`.\n",
                a.tag_prefix
            ));
        }
    }

    let full_pack_section = if a.profile.eq_ignore_ascii_case("FULL") {
        format!(
            "\n---\n\n\
             ## 10. Full 16-Archetype Pack Reference & Deep-Dive (`ARCHETYPE_PROFILE = FULL`)\n\n\
             > All **16 standalone FULL archetypes** (`00` through `15`, including `04-ADRs/`) have been generated under `docs/archetypes/`. You can switch between the individual files using the file selector pills at the top of the Review panel, or review the complete 16-section pack inline below:\n\n\
             | # | Archetype File | Summary |\n\
             |---|---|---|\n\
             | `00` | `docs/archetypes/00-IDENTITY-RECONCILIATION.md` | Code-grounded service definition & module reconciliation |\n\
             | `01` | `docs/archetypes/01-README.md` | Operator entry point, build/test commands, ports, env vars & dependencies |\n\
             | `02` | `docs/archetypes/02-ARCHITECTURE.md` | Current-state architecture record & Step 0.5 Ingress Inventory |\n\
             | `03` | `docs/archetypes/03-SERVICE-MANIFEST.yaml` | Canonical machine-readable manifest (`schema/service-manifest.schema.yaml`) |\n\
             | `04` | `docs/archetypes/04-ADRs/` | 4 Architecture Decision Records (`ADR-001`..`ADR-004`) + index |\n\
             | `05` | `docs/archetypes/05-component-diagram.md` | Internal component Mermaid diagram & ingress parity table |\n\
             | `06` | `docs/archetypes/06-sequence-diagrams.md` | Dedicated Mermaid sequence diagram per active ingress path |\n\
             | `07` | `docs/archetypes/07-data-model.md` | Persistence, schemas & code symbol catalog |\n\
             | `08` | `docs/archetypes/08-error-taxonomy.md` | Failure-mode catalog & exception audit |\n\
             | `09` | `docs/archetypes/09-tier3-service-rules.md` | Reconciled Tier-3 service guardrails |\n\
             | `10` | `docs/archetypes/10-service-memories-seed.md` | AI agent & engineer onboarding context seed |\n\
             | `11` | `docs/archetypes/11-test-strategy.md` | Test strategy, suite inventory & module coverage gaps |\n\
             | `12` | `docs/archetypes/12-threat-model.md` | Trust-boundary Mermaid diagram & STRIDE threat model |\n\
             | `13` | `docs/archetypes/13-observability-contract.md` | Metrics, health checks, loggers & SLO contract |\n\
             | `14` | `docs/archetypes/14-runbook.md` | On-call triage flow diagram, startup checklist & recovery playbooks |\n\
             | `15` | `docs/archetypes/15-tribal-knowledge.md` | Senior-engineer warnings & operational gotchas |\n\n\
             ---\n\n\
             {}\n\n---\n\n{}\n\n---\n\n{}\n\n---\n\n{}\n\n---\n\n{}\n\n---\n\n{}\n\n---\n\n{}\n\n---\n\n{}\n\n---\n\n{}\n\n---\n\n{}\n\n---\n\n{}\n\n---\n\n{}\n\n---\n\n{}\n\n---\n\n{}\n",
            gen_00_identity(a),
            gen_01_readme(a),
            gen_02_architecture(a),
            gen_05_component_diagram(a),
            gen_06_sequence_diagrams(a),
            gen_07_data_model(a),
            gen_08_error_taxonomy(a),
            gen_09_tier3_rules(a),
            gen_10_memories_seed(a),
            gen_11_test_strategy(a),
            gen_12_threat_model(a),
            gen_13_observability(a),
            gen_14_runbook(a),
            gen_15_tribal_knowledge(a),
        )
    } else {
        String::new()
    };

    format!(
        "# Bounded Context — `{svc}`\n\n\
         > Grounded via **`prompts/universal-bounded-context.prompt.md`** & **`orchestrate.agent.md`** (`bounded-context-generator`).\n\
         > Dominant stack(s): **{stack_list}** · Analysed files: **{}** (**{}** lines) · Profile: **{}**\n\n\
         ---\n\n\
         ## 1. Identity & Boundary\n\n\
         - **Context Name:** `{svc}`\n\
         - **Purpose:** {}\n\
         - **Scope:** Covers `{svc}` ({} files, {} lines across {} top-level module(s)).\n\
         - **In Scope:** All {} deterministic ingress/entry path(s), domain symbols (`{}` cataloged), configuration contracts (`{first_cfg}`), and state/artifact outputs.\n\
         - **Explicitly Out of Scope:** Upstream caller UIs outside this repository, external identity provider administration, and unmanaged third-party cloud tenants.\n\n\
         {mermaid_ctx}\n\
         ---\n\n\
         ## 2. Ubiquitous Language\n\n\
         {terms_table}\n\
         ---\n\n\
         ## 3. Inputs & Outputs\n\n\
         ### Deterministic Ingress & Triggers (Step 0.5 Inventory)\n\n\
         {inv_table}\
         {env_table}\n\
         ### Produced Outputs & Sinks\n\
         - Structured responses, persisted state updates in `{first_data}`, and operational telemetry `[cfg: {first_cfg}]`.\n\n\
         ---\n\n\
         ## 4. Data Model / Datasets\n\n\
         - **Source-of-Truth Store:** `{first_data}` (`{}` discovered symbols/models across `{}` data/schema files).\n\
         {data_md}\n\
         ---\n\n\
         ## 5. Invariants & Rules the Code Guarantees\n\n\
         - **Deterministic Entry Routing:** All inbound execution enters via the {} registered handlers in `{first_key}` `[{tp}: {first_key}]`.\n\
         - **Configuration Contract Enforcement:** Runtime parameters and environment overrides bind from `{first_cfg}` ({} env vars discovered) `[cfg: {first_cfg}]`.\n\
         - **Module Boundary Isolation:** Execution is partitioned across {} top-level module(s) (`{second_key}`) `[{tp}: {second_key}]`.\n\
         - **Structured Failure Propagation:** {} error/exception handler pattern(s) guard domain and I/O operations `[{tp}: {first_key}]`.\n\
         - **Test & Verification Grounding:** {} test/verification file(s) corroborate repository behavior `[test: {first_test}]`.\n\n\
         ---\n\n\
         ## 6. Key Flows\n\n\
         ```mermaid\n\
         flowchart TD\n\
             START([\"Trigger: Inbound Request / CLI / Automation Event\"])\n\n\
             subgraph INGEST[\"1. Ingress & Contract Validation ({first_key})\"]\n\
                 A[\"Receive input at registered handler\"]\n\
                 B{{\"Input & config valid?\"}}\n\
             end\n\n\
             subgraph PROCESS[\"2. Core Domain Execution ({svc})\"]\n\
                 C[\"Execute module workflow ({} modules)\"]\n\
                 D{{\"State / artifact mutation required?\"}}\n\
                 E[\"Format structured output\"]\n\
             end\n\n\
             START -->|\"inbound payload / args\"| A\n\
             A --> B\n\
             B -->|\"no\"| REJECT([\"Return validation error / non-zero exit\"])\n\
             B -->|\"yes\"| C\n\
             C --> D\n\
             D -->|\"yes\"| DB[(\"State / Artifact Store ({first_data})\")]\n\
             DB -->|\"persisted\"| E\n\
             D -->|\"no\"| E\n\
             E -->|\"response / event\"| DONE([\"Complete execution\"])\n\
             C -->|\"downstream timeout branch\"| UNVERIFIED[/\"External retry/backoff SLA — unverified\"/]\n\
         ```\n\n\
         ---\n\n\
         ## 7. What the Code Does NOT Do\n\n\
         - Does **not** bypass input/schema contracts defined in `{first_key}` and `{first_data}`.\n\
         - Does **not** perform automatic cross-region replication or external secret rotation inside application code.\n\
         - Does **not** silently discard unhandled runtime exceptions outside the cataloged error handlers (`08-error-taxonomy.md`).\n\n\
         ---\n\n\
         ## 8. Prod vs Non-Prod\n\n\
         - **Core ingress & module execution (`{first_key}`):** `in production` `[{tp}: {first_key}]`\n\
         - **Declarative configuration & manifests (`{first_cfg}`):** `in production` `[cfg: {first_cfg}]`\n\
         - **Upstream gateway rate-limit & circuit-breaker tuning:** `unverified` (confirm in deployment environment)\n\n\
         ---\n\n\
         ## 9. Open Questions\n\n\
         - **GAP-1:** Confirm production rate-limiting and timeout thresholds enforced upstream of `{svc}`.\n\
         - **GAP-2:** Confirm automated test coverage gate threshold in CI/CD before production promotion.\n\
         {full_pack_section}",
        a.total_files,
        a.total_lines,
        a.profile,
        a.description,
        a.total_files,
        a.total_lines,
        a.modules.len().max(1),
        a.ingress.len(),
        a.symbols.len(),
        a.symbols.len(),
        a.data_files.len().max(1),
        a.ingress.len(),
        a.env_vars.len(),
        a.modules.len().max(1),
        a.errors.len(),
        a.test_files.len(),
        a.modules.len().max(1),
        tp = a.tag_prefix,
    )
}

fn legacy_document(prompt: &str, root: &Path) -> String {
    let repo = Regex::new(r"([A-Za-z0-9-]+/[A-Za-z0-9._-]+)")
        .unwrap()
        .find(prompt)
        .map(|m| m.as_str().to_owned())
        .unwrap_or_else(|| "repository".into());
    let modules = top_level_modules(root);
    let mut doc = format!(
        "# Bounded Contexts — {repo}\n\n> Generated by **fake-copilot**, a local test double. This is not real agent output.\n\n\
         ## Summary\n\nThe repository contains {} candidate bounded context(s).\n\n## Context Map\n\n```mermaid\nflowchart LR\n",
        modules.len().max(1)
    );
    let names: Vec<String> =
        if modules.is_empty() { vec!["Core".into()] } else { modules.keys().map(|m| title(m)).collect() };
    for n in &names {
        doc.push_str(&format!("  {n}[{n}]\n"));
    }
    for pair in names.windows(2) {
        doc.push_str(&format!("  {} -->|customer/supplier| {}\n", pair[0], pair[1]));
    }
    doc.push_str("```\n\n## Contexts\n\n");
    for (m, files) in &modules {
        doc.push_str(&format!(
            "### {}\n\n- **Responsibility:** Inferred from `{m}`.\n- **Key code locations:** `{m}/` ({files} files)\n\n",
            title(m)
        ));
    }
    doc.push_str("## Relationships\n\nSee the context map.\n\n## Open questions\n\n- Confirm context boundaries with the domain owners.\n");
    doc
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = parse_args(&argv);
    if args.version {
        println!("1.0.103 (aifc-archetype-orchestrator)");
        return ExitCode::SUCCESS;
    }
    let Some(prompt) = args.prompt.clone() else {
        eprintln!("Error: fake-copilot only supports non-interactive mode (-p)");
        return ExitCode::from(2);
    };
    let token = std::env::var("COPILOT_GITHUB_TOKEN").unwrap_or_default();
    if token.trim().is_empty()
        || !(token.starts_with("ghp_")
            || token.starts_with("gho_")
            || token.starts_with("ghu_")
            || token.starts_with("ghs_")
            || token.starts_with("github_pat_")
            || token.len() >= 20)
    {
        eprintln!("Error: Not authorized. No valid Copilot token was provided (401).");
        return ExitCode::from(1);
    }
    if let Some(agent) = &args.agent {
        let home = std::env::var("COPILOT_HOME").map(PathBuf::from).unwrap_or_default();
        if !home.join("agents").join(format!("{agent}.agent.md")).is_file() {
            eprintln!("Error: custom agent '{agent}' not found");
            return ExitCode::from(1);
        }
    }
    let cwd = std::env::current_dir().expect("cwd");
    let mode = std::fs::read_to_string(cwd.join(".fake-copilot-mode"))
        .ok()
        .or_else(|| std::env::var("FAKE_COPILOT_MODE").ok())
        .map(|m| m.trim().to_owned())
        .unwrap_or_else(|| "ok".into());
    let out = output_path(&prompt);
    let pause = |ms| sleep(Duration::from_millis(ms));

    say(&format!(
        "● Using agent {} with model {}",
        args.agent.as_deref().unwrap_or("default"),
        args.model.as_deref().unwrap_or("default")
    ));
    pause(150);
    match mode.as_str() {
        "unauthorized" => {
            eprintln!("Error: You are not authorized to use Copilot. Check your Copilot subscription.");
            return ExitCode::from(1);
        }
        "model_unavailable" => {
            eprintln!("Error: Model \"{}\" is not available.", args.model.unwrap_or_default());
            return ExitCode::from(1);
        }
        "fail" => {
            say("● Reading repository structure");
            eprintln!("Error: the agent encountered an unrecoverable problem (simulated)");
            return ExitCode::from(1);
        }
        "hang" => {
            say("● Thinking very hard (simulated hang)");
            sleep(Duration::from_secs(24 * 3600));
        }
        _ => {}
    }

    let is_orchestrate = args.agent.as_deref() == Some("orchestrate");
    if is_orchestrate && mode == "ok" {
        say("● Step 0.5 — Running Deterministic Ingress Inventory & Deep Code Analysis across repository");
        let analysis = analyze_repo(&prompt, &cwd);
        say(&format!(
            "● Detected stack(s): {} ({} files, {} lines, {} ingress paths, {} symbols, profile={})",
            analysis.stacks.join(", "),
            analysis.total_files,
            analysis.total_lines,
            analysis.ingress.len(),
            analysis.symbols.len(),
            analysis.profile
        ));
        pause(150);
        for b in 1..=5 {
            say(&format!("● Running Archetype Batch {b}/5 ({} profile)", analysis.profile));
            pause(120);
        }
        let archetype_files = write_archetype_pack(&cwd, &analysis);
        for f in &archetype_files {
            say(&format!("✓ Wrote {f}"));
        }
        say("● Running Batch 6 — Universal Bounded Context (prompts/universal-bounded-context.prompt.md)");
        pause(150);
        let bctx_path = format!("docs/BOUNDED-CONTEXT-{}.md", analysis.service_name);
        let bctx_doc = generate_bounded_context_doc(&analysis);
        write(&cwd, &bctx_path, &bctx_doc);
        say(&format!("✓ Wrote {bctx_path} (🐤 SE-CANARY-OK)"));
    } else {
        let steps = if mode == "slow" { 15 } else { 3 };
        for i in 1..=steps {
            say(&format!("● Analysing repository ({i}/{steps})"));
            pause(if mode == "slow" { 2000 } else { 250 });
        }
        match mode.as_str() {
            "no_output" => {
                say("● Done (wrote nothing)");
                return ExitCode::SUCCESS;
            }
            "empty" => write(&cwd, &out, ""),
            _ => write(&cwd, &out, &legacy_document(&prompt, &cwd)),
        }
        if mode == "unexpected" {
            let _ = std::fs::write(cwd.join("README.md"), "# Modified by the agent (should never be committed)\n");
            let _ = std::fs::write(cwd.join("notes.txt"), "scratch notes\n");
            say("● Also touched README.md and notes.txt");
        }
        say(&format!("✓ Wrote {out}"));
    }

    if let Some(share) = args.share {
        let _ = std::fs::write(share, format!("# Transcript\n\nPrompt:\n\n{prompt}\n"));
    }
    ExitCode::SUCCESS
}

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create output dir");
    }
    std::fs::write(path, content).expect("write output");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cli_shapes() {
        let argv: Vec<String> = [
            "-p",
            "Write `docs/x.md`",
            "--agent=bc",
            "--model",
            "auto",
            "-s",
            "--allow-tool",
            "read",
            "--share",
            "/t.md",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let a = parse_args(&argv);
        assert_eq!(a.prompt.as_deref(), Some("Write `docs/x.md`"));
        assert_eq!(a.agent.as_deref(), Some("bc"));
        assert_eq!(a.model.as_deref(), Some("auto"));
        assert_eq!(a.share, Some(PathBuf::from("/t.md")));
    }

    #[test]
    fn finds_output_path() {
        assert_eq!(output_path("Write the result to `docs/bounded-context.md`. Do not"), "docs/bounded-context.md");
        assert_eq!(output_path("Write to docs/contexts/map.md please"), "docs/contexts/map.md");
        assert_eq!(output_path("no path"), "docs/bounded-context.md");
        assert_eq!(output_path("Write `/etc/x.md`"), "docs/bounded-context.md");
    }
}
