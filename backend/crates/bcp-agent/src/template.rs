//! Minimal logic-less template subset for agent prompts: `{{name}}`, sections `{{#name}}…{{/name}}`
//! (rendered when the value is non-empty) and inverted sections `{{^name}}…{{/name}}`. No HTML escaping —
//! the output is a CLI argument, never markup — and no partials, lambdas or nested lookups.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Result};

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Text(String),
    Var(String),
    Section { name: String, inverted: bool, children: Vec<Node> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    nodes: Vec<Node>,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl Template {
    pub fn parse(src: &str) -> Result<Self> {
        let mut stack: Vec<(String, bool, Vec<Node>)> = Vec::new();
        let mut current: Vec<Node> = Vec::new();
        let mut rest = src;
        while let Some(start) = rest.find("{{") {
            if start > 0 {
                current.push(Node::Text(rest[..start].to_owned()));
            }
            let after = &rest[start + 2..];
            let Some(end) = after.find("}}") else { bail!("unclosed '{{{{' in prompt template") };
            let tag = after[..end].trim();
            rest = &after[end + 2..];
            let (sigil, name) = match tag.chars().next() {
                Some(c @ ('#' | '^' | '/')) => (Some(c), tag[1..].trim()),
                _ => (None, tag),
            };
            if !valid_name(name) {
                bail!("invalid template variable {tag:?} (use lowercase letters, digits and '_')");
            }
            match sigil {
                None => current.push(Node::Var(name.to_owned())),
                Some(c @ ('#' | '^')) => {
                    stack.push((name.to_owned(), c == '^', std::mem::take(&mut current)));
                }
                Some(_) => {
                    let Some((open, inverted, parent)) = stack.pop() else {
                        bail!("closing tag {{{{/{name}}}}} without an opening tag");
                    };
                    if open != name {
                        bail!("section {{{{#{open}}}}} closed by {{{{/{name}}}}}");
                    }
                    let children = std::mem::replace(&mut current, parent);
                    current.push(Node::Section { name: open, inverted, children });
                }
            }
        }
        if !rest.is_empty() {
            current.push(Node::Text(rest.to_owned()));
        }
        if let Some((open, _, _)) = stack.pop() {
            bail!("section {{{{#{open}}}}} is never closed");
        }
        Ok(Self { nodes: current })
    }

    /// Every variable name referenced, including section names.
    pub fn variables(&self) -> BTreeSet<String> {
        fn walk(nodes: &[Node], out: &mut BTreeSet<String>) {
            for n in nodes {
                match n {
                    Node::Text(_) => {}
                    Node::Var(name) => {
                        out.insert(name.clone());
                    }
                    Node::Section { name, children, .. } => {
                        out.insert(name.clone());
                        walk(children, out);
                    }
                }
            }
        }
        let mut out = BTreeSet::new();
        walk(&self.nodes, &mut out);
        out
    }

    /// Renders with `vars`; missing variables render as empty strings (unknown names are rejected at load).
    pub fn render(&self, vars: &BTreeMap<String, String>) -> String {
        fn walk(nodes: &[Node], vars: &BTreeMap<String, String>, out: &mut String) {
            for n in nodes {
                match n {
                    Node::Text(t) => out.push_str(t),
                    Node::Var(name) => out.push_str(vars.get(name).map(String::as_str).unwrap_or_default()),
                    Node::Section { name, inverted, children } => {
                        let truthy = vars.get(name).is_some_and(|v| !v.trim().is_empty());
                        if truthy != *inverted {
                            walk(children, vars, out);
                        }
                    }
                }
            }
        }
        let mut out = String::new();
        walk(&self.nodes, vars, &mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn renders_vars_and_sections() {
        let t = Template::parse(
            "Repo {{owner}}/{{repo}}.{{#scope}} Scope: {{scope}}.{{/scope}}{{^scope}} Whole repo.{{/scope}}",
        )
        .unwrap();
        assert_eq!(t.render(&vars(&[("owner", "acme"), ("repo", "shop")])), "Repo acme/shop. Whole repo.");
        assert_eq!(
            t.render(&vars(&[("owner", "acme"), ("repo", "shop"), ("scope", "svc/a")])),
            "Repo acme/shop. Scope: svc/a."
        );
        assert_eq!(t.variables().into_iter().collect::<Vec<_>>(), ["owner", "repo", "scope"]);
    }

    #[test]
    fn values_are_not_reinterpreted() {
        let t = Template::parse("{{x}}").unwrap();
        assert_eq!(t.render(&vars(&[("x", "{{owner}}")])), "{{owner}}");
    }

    #[test]
    fn rejects_malformed_templates() {
        for bad in ["{{unclosed", "{{#a}}x", "{{/a}}", "{{#a}}{{/b}}", "{{Bad}}", "{{a.b}}", "{{}}"] {
            assert!(Template::parse(bad).is_err(), "{bad}");
        }
    }
}
