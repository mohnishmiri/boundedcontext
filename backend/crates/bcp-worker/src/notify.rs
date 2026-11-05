//! Completion email: tells the requester their bounded-context documentation is ready, lists the generated
//! documents (title and sections) and links back to the portal. Best effort: mail problems never affect the job.

use std::fmt::Write as _;
use std::time::Duration;

use anyhow::Context;
use bcp_github::encode_path;
use bcp_store::{ArtifactRow, JobRow, UserRow};
use chrono::{DateTime, Utc};
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use uuid::Uuid;

use crate::config::SmtpConfig;
use crate::logger::JobLogger;
use crate::Worker;

const LOGO_PNG: &[u8] = include_bytes!("../assets/att-logo.png");
const LOGO_CID: &str = "att-logo";
const SEND_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_DOCUMENTS: usize = 6;
const MAX_SECTIONS: usize = 15;
const MAX_OTHER_FILES: usize = 25;

pub struct Mailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    fallback_domain: Option<String>,
    cc_admin_logins: Vec<String>,
}

impl Mailer {
    pub fn new(cfg: &SmtpConfig) -> anyhow::Result<Self> {
        let address: Address = cfg.from_address.parse().context("SMTP_FROM_ADDRESS is not a valid email address")?;
        let mut builder = if !cfg.use_tls {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(cfg.host.as_str())
        } else if cfg.port == 465 {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host)?
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host)?
        };
        builder = builder.port(cfg.port).timeout(Some(Duration::from_secs(30)));
        if let (Some(user), Some(password)) = (&cfg.username, &cfg.password) {
            builder = builder.credentials(Credentials::new(user.clone(), password.expose().to_owned()));
        }
        Ok(Self {
            transport: builder.build(),
            from: Mailbox::new(Some(cfg.from_name.clone()), address),
            fallback_domain: cfg.fallback_domain.clone(),
            cc_admin_logins: cfg.cc_admin_logins.clone(),
        })
    }
}

impl Worker {
    /// Emails the requester that generation finished. Outcomes are recorded in the job log, never as errors.
    pub(crate) async fn notify_completed(&self, job_id: Uuid, log: &JobLogger) {
        let Some(mailer) = &self.mailer else { return };
        let outcome = tokio::time::timeout(SEND_TIMEOUT, self.send_completion(mailer, job_id))
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("timed out after {}s", SEND_TIMEOUT.as_secs())));
        match outcome {
            Ok(Some((to, 0))) => {
                metrics::counter!("bcp_notifications_total", "outcome" => "sent").increment(1);
                log.info(&format!("Sent the completion email to {to}")).await;
            }
            Ok(Some((to, cc))) => {
                metrics::counter!("bcp_notifications_total", "outcome" => "sent").increment(1);
                log.info(&format!("Sent the completion email to {to} (copied to {cc} administrator(s))")).await;
            }
            Ok(None) => {
                metrics::counter!("bcp_notifications_total", "outcome" => "no_recipient").increment(1);
                log.warn(
                    "No completion email was sent: GitHub does not share an email address for your account. Make \
                     your email public on GitHub, or sign in again with the user:email permission.",
                )
                .await;
            }
            Err(e) => {
                metrics::counter!("bcp_notifications_total", "outcome" => "failed").increment(1);
                tracing::warn!(%job_id, error = %format!("{e:#}"), "completion email failed");
                log.warn(
                    "The completion email could not be sent. The platform team can find details in the worker logs.",
                )
                .await;
            }
        }
    }

    /// The recipient's address and the number of administrators copied, or `None` without a recipient.
    async fn send_completion(&self, mailer: &Mailer, job_id: Uuid) -> anyhow::Result<Option<(String, usize)>> {
        let Some(job) = self.store.get_job(job_id).await? else { return Ok(None) };
        let user = self.store.get_user(job.user_id).await?;
        let Some((to, github_name)) = self.address_of(mailer, Some(job.user_id), &job.requested_by).await else {
            return Ok(None);
        };
        let cc = self.admin_cc(mailer, &job, &to).await;
        let name = user
            .as_ref()
            .and_then(|u: &UserRow| u.name.clone())
            .or(github_name)
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| job.requested_by.clone());
        let artifacts = self.store.list_artifacts(job.id).await?;
        let report =
            Report::new(&job, &artifacts, name, self.cfg.common.public_url.as_str(), self.github.web_url().as_str());
        let logo = Attachment::new_inline(LOGO_CID.to_owned())
            .body(LOGO_PNG.to_vec(), ContentType::parse("image/png").expect("valid content type"));
        let mut builder = Message::builder().from(mailer.from.clone()).to(to.clone()).subject(report.subject());
        for m in &cc {
            builder = builder.cc(m.clone());
        }
        let message =
            builder
                .multipart(MultiPart::alternative().singlepart(SinglePart::plain(render_text(&report))).multipart(
                    MultiPart::related().singlepart(SinglePart::html(render_html(&report))).singlepart(logo),
                ))
                .context("building the email")?;
        mailer.transport.send(message).await.context("sending the email")?;
        Ok(Some((to.email.to_string(), cc.len())))
    }

    /// Addresses of the administrators to copy, excluding the recipient and duplicates.
    async fn admin_cc(&self, mailer: &Mailer, job: &JobRow, to: &Mailbox) -> Vec<Mailbox> {
        let mut out: Vec<Mailbox> = Vec::new();
        for login in &mailer.cc_admin_logins {
            let user_id = match self.store.get_user_by_login(login).await {
                Ok(u) => u.map(|u| u.id),
                Err(e) => {
                    tracing::debug!(job_id = %job.id, %login, error = %e, "looking up the administrator failed");
                    None
                }
            };
            match self.address_of(mailer, user_id, login).await {
                Some((m, _)) => {
                    let same = |a: &Mailbox| a.email.to_string().eq_ignore_ascii_case(&m.email.to_string());
                    if !same(to) && !out.iter().any(same) {
                        out.push(m);
                    }
                }
                None => tracing::warn!(job_id = %job.id, %login, "no email address for administrator; not copied"),
            }
        }
        out
    }

    /// A user's address: public GitHub email, else their primary verified one (both need a stored token, i.e. a
    /// user who signed in to the portal), else `<login>@<fallback domain>`.
    async fn address_of(
        &self,
        mailer: &Mailer,
        user_id: Option<Uuid>,
        login: &str,
    ) -> Option<(Mailbox, Option<String>)> {
        let mut github_name = None;
        let token = match user_id {
            Some(id) => self.tokens.access_token(id).await.ok(),
            None => None,
        };
        if let Some(token) = token {
            match self.github.get_authenticated_user(&token).await {
                Ok(u) => {
                    github_name = u.name.clone();
                    if let Some(addr) = u.email.as_deref().and_then(|e| e.parse::<Address>().ok()) {
                        return Some((Mailbox::new(github_name.clone(), addr), github_name));
                    }
                }
                Err(e) => tracing::debug!(%login, error = %e, "reading the GitHub profile failed"),
            }
            match self.github.list_user_emails(&token).await {
                Ok(emails) => {
                    let pick = emails
                        .iter()
                        .find(|e| e.primary && e.verified)
                        .or_else(|| emails.iter().find(|e| e.verified && !e.email.ends_with("noreply.github.com")));
                    if let Some(addr) = pick.and_then(|e| e.email.parse::<Address>().ok()) {
                        return Some((Mailbox::new(github_name.clone(), addr), github_name));
                    }
                }
                Err(e) => tracing::debug!(%login, error = %e, "listing GitHub emails failed"),
            }
        }
        let domain = mailer.fallback_domain.as_deref()?;
        let addr = fallback_address(login, domain)?;
        Some((Mailbox::new(github_name.clone(), addr), github_name))
    }
}

/// `<login>@<domain>`, dropping the `_shortcode` suffix of enterprise managed users (GitHub logins never
/// contain `_` otherwise).
fn fallback_address(login: &str, domain: &str) -> Option<Address> {
    let handle = login.rsplit_once('_').map_or(login, |(h, _)| h);
    if handle.is_empty() || !handle.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    Address::new(handle.to_ascii_lowercase(), domain).ok()
}

// ------------------------------------------------------------------------------------------------------
// Content
// ------------------------------------------------------------------------------------------------------

struct Document {
    path: String,
    title: Option<String>,
    sections: Vec<String>,
    change: &'static str,
    size: i64,
}

struct Report {
    name: String,
    repo: String,
    repo_url: String,
    branch: String,
    branch_url: String,
    commit: Option<(String, String)>,
    generator: String,
    model: String,
    inputs: Vec<(String, String)>,
    requested_by: String,
    started: Option<DateTime<Utc>>,
    finished: Option<DateTime<Utc>>,
    job_id: String,
    job_url: String,
    documents: Vec<Document>,
    more_documents: usize,
    others: Vec<(String, &'static str)>,
    more_others: usize,
    total: usize,
    added: usize,
    ignored: usize,
}

impl Report {
    fn new(job: &JobRow, artifacts: &[ArtifactRow], name: String, portal: &str, web: &str) -> Self {
        let portal = portal.trim_end_matches('/');
        let repo_url = format!("{}/{}/{}", web.trim_end_matches('/'), encode_path(&job.owner), encode_path(&job.repo));
        let kept: Vec<&ArtifactRow> = artifacts.iter().filter(|a| a.allowed && a.new_content.is_some()).collect();
        let primary = job.output_primary.as_deref();
        let (key, rest): (Vec<&ArtifactRow>, Vec<&ArtifactRow>) =
            kept.iter().copied().partition(|a| Some(a.path.as_str()) == primary || is_key_document(&a.path));
        let documents = key
            .iter()
            .take(MAX_DOCUMENTS)
            .map(|a| {
                let (title, sections) = outline(a.new_content.as_deref().unwrap_or_default());
                Document {
                    path: a.path.clone(),
                    title,
                    sections,
                    change: change_label(&a.change_type),
                    size: a.size_bytes,
                }
            })
            .collect();
        let others =
            rest.iter().take(MAX_OTHER_FILES).map(|a| (a.path.clone(), change_label(&a.change_type))).collect();
        let inputs =
            serde_json::from_value::<std::collections::BTreeMap<String, serde_json::Value>>(job.inputs.clone())
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(k, v)| {
                    let value = match v {
                        serde_json::Value::String(s) => s,
                        serde_json::Value::Null => return None,
                        other => other.to_string(),
                    };
                    (!value.trim().is_empty()).then(|| (humanize(&k), value))
                })
                .collect();
        Self {
            name,
            repo: format!("{}/{}", job.owner, job.repo),
            branch_url: format!("{repo_url}/tree/{}", encode_path(&job.base_branch)),
            commit: job
                .base_sha
                .as_ref()
                .map(|sha| (sha[..sha.len().min(12)].to_owned(), format!("{repo_url}/commit/{sha}"))),
            repo_url,
            branch: job.base_branch.clone(),
            generator: match &job.generator_name {
                Some(n) if !n.is_empty() => format!("{n} (bundle {})", job.agent_version),
                _ => format!("{} (bundle {})", job.agent_id, job.agent_version),
            },
            model: job.model.clone(),
            inputs,
            requested_by: job.requested_by.clone(),
            started: job.started_at,
            finished: job.finished_at,
            job_id: job.short_id(),
            job_url: format!("{portal}/jobs/{}", job.id),
            more_documents: key.len().saturating_sub(MAX_DOCUMENTS),
            documents,
            more_others: rest.len().saturating_sub(MAX_OTHER_FILES),
            others,
            total: kept.len(),
            added: kept.iter().filter(|a| a.change_type == "added").count(),
            ignored: artifacts.iter().filter(|a| !a.allowed).count(),
        }
    }

    fn subject(&self) -> String {
        format!("Bounded context documentation ready: {} ({})", self.repo, self.branch)
    }

    fn files_sentence(&self) -> String {
        match self.total {
            0 => "No files were produced.".to_owned(),
            1 => "1 file is ready for your review.".to_owned(),
            n => format!("{n} files are ready for your review ({} new, {} updated).", self.added, n - self.added),
        }
    }

    fn duration(&self) -> Option<String> {
        let secs = (self.finished? - self.started?).num_seconds().max(0);
        Some(match (secs / 3600, secs % 3600 / 60, secs % 60) {
            (0, 0, s) => format!("{s} s"),
            (0, m, s) => format!("{m} min {s} s"),
            (h, m, _) => format!("{h} h {m} min"),
        })
    }
}

fn is_key_document(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let file = lower.rsplit('/').next().unwrap_or(&lower);
    lower.ends_with(".md")
        && !lower.contains("/archetypes/")
        && (file.starts_with("bounded-context")
            || file.starts_with("infra-context")
            || lower.contains("/bounded-contexts/"))
}

fn change_label(change: &str) -> &'static str {
    match change {
        "added" => "New",
        "deleted" => "Deleted",
        _ => "Updated",
    }
}

fn humanize(key: &str) -> String {
    let words = key.replace(['_', '-'], " ");
    let mut chars = words.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// The first `#` heading and the `##` headings of a Markdown document, outside code fences.
fn outline(markdown: &str) -> (Option<String>, Vec<String>) {
    let (mut title, mut sections, mut fenced) = (None, Vec::new(), false);
    for line in markdown.lines() {
        let line = line.trim_end();
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if let Some(t) = line.strip_prefix("# ") {
            if title.is_none() {
                title = Some(clean_heading(t)).filter(|t| !t.is_empty());
            }
        } else if let Some(s) = line.strip_prefix("## ") {
            let s = clean_heading(s);
            if !s.is_empty() && sections.len() < MAX_SECTIONS {
                sections.push(s);
            }
        }
    }
    (title, sections)
}

/// Plain text of a heading: no emphasis, code ticks, trailing `#`s or link targets; at most 120 characters.
fn clean_heading(raw: &str) -> String {
    let mut text = String::new();
    let mut rest = raw.trim().trim_end_matches('#').trim();
    while let Some(i) = rest.find("](") {
        text.push_str(&rest[..i]);
        rest = rest[i + 2..].find(')').map_or("", |j| &rest[i + 2 + j + 1..]);
    }
    text.push_str(rest);
    let text: String = text.chars().filter(|c| !matches!(c, '*' | '`' | '[')).collect();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > 120 {
        format!("{}…", text.chars().take(119).collect::<String>())
    } else {
        text
    }
}

fn human_size(bytes: i64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / 1_048_576.0),
    }
}

fn when(t: Option<DateTime<Utc>>) -> Option<String> {
    t.map(|t| t.format("%d %b %Y, %H:%M UTC").to_string())
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

// ------------------------------------------------------------------------------------------------------
// Rendering (table layout and inline styles for Outlook and other mail clients)
// ------------------------------------------------------------------------------------------------------

const FONT: &str = "font-family:Arial,Helvetica,sans-serif;";
const MONO: &str = "font-family:Consolas,'Courier New',monospace;";
const LINK: &str = "color:#067ab4;text-decoration:none;";

fn link(url: &str, text: &str) -> String {
    format!(r#"<a href="{}" target="_blank" style="{LINK}">{}</a>"#, esc(url), esc(text))
}

fn section_title(h: &mut String, text: &str) {
    let _ = write!(
        h,
        r#"<tr><td style="padding:28px 32px 4px;"><h2 style="margin:0;padding-bottom:8px;border-bottom:1px solid #e6eaef;{FONT}font-size:16px;line-height:22px;color:#033e5d;">{text}</h2></td></tr>"#
    );
}

fn detail_row(h: &mut String, label: &str, value_html: &str) {
    let _ = write!(
        h,
        r#"<tr><td width="34%" valign="top" style="padding:7px 12px 7px 0;{FONT}font-size:13px;line-height:19px;color:#5a6772;">{label}</td><td valign="top" style="padding:7px 0;{FONT}font-size:13px;line-height:19px;color:#1d2329;">{value_html}</td></tr>"#
    );
}

fn render_html(r: &Report) -> String {
    let mut h = String::with_capacity(16 * 1024);
    let year = Utc::now().format("%Y");
    let _ = write!(
        h,
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="x-apple-disable-message-reformatting">
<title>{title}</title>
</head>
<body style="margin:0;padding:0;background-color:#f2f4f7;">
<div style="display:none;max-height:0;overflow:hidden;mso-hide:all;font-size:1px;line-height:1px;color:#f2f4f7;">{preheader}</div>
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="background-color:#f2f4f7;">
<tr><td align="center" style="padding:24px 12px;">
<table role="presentation" width="640" cellpadding="0" cellspacing="0" border="0" style="width:100%;max-width:640px;background-color:#ffffff;border:1px solid #dde3ea;">
<tr><td style="padding:20px 32px;border-bottom:4px solid #00a8e0;">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0"><tr>
<td align="left" valign="middle"><img src="cid:{LOGO_CID}" width="112" height="46" alt="AT&amp;T" style="display:block;border:0;outline:none;text-decoration:none;"></td>
<td align="right" valign="middle" style="{FONT}font-size:13px;font-weight:bold;color:#067ab4;">Bounded Context Portal</td>
</tr></table>
</td></tr>
<tr><td style="padding:28px 32px 0;">
<span style="display:inline-block;padding:4px 10px;border-radius:12px;background-color:#e6f4ea;{FONT}color:#1b7a3a;font-size:11px;font-weight:bold;letter-spacing:0.5px;">GENERATION COMPLETED</span>
<h1 style="margin:14px 0 14px;{FONT}font-size:22px;line-height:28px;color:#033e5d;font-weight:bold;">Your bounded context documentation is ready</h1>
<p style="margin:0 0 12px;{FONT}font-size:14px;line-height:22px;color:#1d2329;">Hello {name},</p>
<p style="margin:0;{FONT}font-size:14px;line-height:22px;color:#1d2329;">The Bounded Context Portal has finished analysing <a href="{repo_url}" target="_blank" style="{LINK}font-weight:bold;">{repo}</a> on branch <strong>{branch}</strong>. {files}</p>
</td></tr>
<tr><td style="padding:24px 32px 0;">
<table role="presentation" cellpadding="0" cellspacing="0" border="0"><tr>
<td align="center" bgcolor="#067ab4" style="border-radius:4px;background-color:#067ab4;">
<a href="{job_url}" target="_blank" style="display:inline-block;padding:12px 28px;{FONT}font-size:14px;font-weight:bold;color:#ffffff;text-decoration:none;border-radius:4px;">Review documentation in the portal</a>
</td></tr></table>
</td></tr>
"##,
        title = esc(&r.subject()),
        preheader = esc(&format!("{} on {}: {}", r.repo, r.branch, r.files_sentence())),
        name = esc(&r.name),
        repo_url = esc(&r.repo_url),
        repo = esc(&r.repo),
        branch = esc(&r.branch),
        files = esc(&r.files_sentence()),
        job_url = esc(&r.job_url),
    );

    // ---- summary
    section_title(&mut h, "Job summary");
    h.push_str(r#"<tr><td style="padding:4px 32px 0;"><table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0">"#);
    detail_row(&mut h, "Repository", &link(&r.repo_url, &r.repo));
    detail_row(&mut h, "Branch", &link(&r.branch_url, &r.branch));
    if let Some((short, url)) = &r.commit {
        detail_row(&mut h, "Analysed commit", &format!(r#"<span style="{MONO}">{}</span>"#, link(url, short)));
    }
    detail_row(&mut h, "Generator", &esc(&r.generator));
    if !r.model.is_empty() {
        detail_row(&mut h, "Model", &esc(&r.model));
    }
    for (label, value) in &r.inputs {
        detail_row(&mut h, &esc(label), &esc(value));
    }
    detail_row(&mut h, "Requested by", &esc(&r.requested_by));
    if let Some(t) = when(r.started) {
        detail_row(&mut h, "Started", &esc(&t));
    }
    if let Some(t) = when(r.finished) {
        detail_row(&mut h, "Completed", &esc(&t));
    }
    if let Some(d) = r.duration() {
        detail_row(&mut h, "Duration", &esc(&d));
    }
    detail_row(&mut h, "Job", &format!(r#"<span style="{MONO}">{}</span>"#, link(&r.job_url, &r.job_id)));
    h.push_str("</table></td></tr>\n");

    // ---- bounded context documents
    if !r.documents.is_empty() {
        section_title(&mut h, "Bounded context documents");
        for d in &r.documents {
            let title = d.title.as_deref().unwrap_or_else(|| d.path.rsplit('/').next().unwrap_or(&d.path));
            let _ = write!(
                h,
                r#"<tr><td style="padding:10px 32px 0;">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="border:1px solid #dde3ea;border-left:4px solid #00a8e0;background-color:#f7fbfd;">
<tr><td style="padding:14px 16px;">
<div style="{FONT}font-size:15px;line-height:21px;font-weight:bold;color:#033e5d;">{title}</div>
<div style="margin-top:4px;{MONO}font-size:12px;line-height:17px;color:#5a6772;word-break:break-all;">{path}</div>
<div style="margin-top:4px;{FONT}font-size:12px;line-height:17px;color:#5a6772;">{change} &middot; {size}{count}</div>"#,
                title = esc(title),
                path = esc(&d.path),
                change = d.change,
                size = human_size(d.size),
                count = if d.sections.is_empty() {
                    String::new()
                } else {
                    format!(" &middot; {} sections", d.sections.len())
                },
            );
            if !d.sections.is_empty() {
                let _ = write!(
                    h,
                    r#"<ol style="margin:10px 0 0;padding-left:22px;{FONT}font-size:13px;line-height:20px;color:#1d2329;">"#
                );
                for s in &d.sections {
                    let _ = write!(h, "<li>{}</li>", esc(s));
                }
                h.push_str("</ol>");
            }
            h.push_str("</td></tr></table>\n</td></tr>\n");
        }
        if r.more_documents > 0 {
            let _ = write!(
                h,
                r#"<tr><td style="padding:8px 32px 0;{FONT}font-size:13px;color:#5a6772;">…and {} more document(s) in the portal.</td></tr>"#,
                r.more_documents
            );
        }
    }

    // ---- other files
    if !r.others.is_empty() {
        section_title(&mut h, if r.documents.is_empty() { "Generated files" } else { "Supporting files" });
        let _ = write!(
            h,
            r#"<tr><td style="padding:8px 32px 0;"><table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0">"#
        );
        for (path, change) in &r.others {
            let _ = write!(
                h,
                r#"<tr><td valign="top" style="padding:3px 8px 3px 0;{MONO}font-size:12px;line-height:17px;color:#1d2329;word-break:break-all;">{}</td><td valign="top" align="right" style="padding:3px 0;{FONT}font-size:12px;line-height:17px;color:#5a6772;white-space:nowrap;">{change}</td></tr>"#,
                esc(path)
            );
        }
        if r.more_others > 0 {
            let _ = write!(
                h,
                r#"<tr><td colspan="2" style="padding:6px 0 0;{FONT}font-size:13px;color:#5a6772;">…and {} more file(s) in the portal.</td></tr>"#,
                r.more_others
            );
        }
        h.push_str("</table></td></tr>\n");
    }
    if r.ignored > 0 {
        let _ = write!(
            h,
            r#"<tr><td style="padding:12px 32px 0;{FONT}font-size:12px;line-height:18px;color:#8a5a00;">{} change(s) outside the allowed output paths were ignored and will never be committed.</td></tr>"#,
            r.ignored
        );
    }

    // ---- next steps
    section_title(&mut h, "Next steps");
    let _ = write!(
        h,
        r#"<tr><td style="padding:8px 32px 0;"><ol style="margin:0;padding-left:22px;{FONT}font-size:13px;line-height:21px;color:#1d2329;">
<li>Open the job in the portal to preview each document and compare it with the repository.</li>
<li>Check the content carefully: it was generated by an AI agent and may contain inaccuracies.</li>
<li>When you are satisfied, open a pull request from the portal. Pull requests are never merged automatically.</li>
</ol></td></tr>
<tr><td style="padding:28px 32px 0;"></td></tr>
<tr><td style="padding:20px 32px;background-color:#f7f9fb;border-top:1px solid #e6eaef;{FONT}font-size:11px;line-height:17px;color:#6b7785;">
This is an automated message from the AT&amp;T Bounded Context Portal, sent because you requested this job or administer the portal. Please do not reply to this email.<br><br>
&copy; {year} AT&amp;T Intellectual Property. All rights reserved.<br>
AT&amp;T Proprietary (Internal Use Only). Not for use or disclosure outside the AT&amp;T companies except under written agreement.
</td></tr>
</table>
</td></tr>
</table>
</body>
</html>
"#
    );
    h
}

fn render_text(r: &Report) -> String {
    let mut t = String::with_capacity(4 * 1024);
    let _ = writeln!(t, "AT&T Bounded Context Portal\n");
    let _ = writeln!(t, "YOUR BOUNDED CONTEXT DOCUMENTATION IS READY\n");
    let _ = writeln!(t, "Hello {},\n", r.name);
    let _ = writeln!(
        t,
        "The Bounded Context Portal has finished analysing {} on branch {}. {}\n",
        r.repo,
        r.branch,
        r.files_sentence()
    );
    let _ = writeln!(t, "Review the documentation: {}\n", r.job_url);
    let _ = writeln!(t, "JOB SUMMARY");
    let _ = writeln!(t, "  Repository:      {} ({})", r.repo, r.repo_url);
    let _ = writeln!(t, "  Branch:          {}", r.branch);
    if let Some((short, _)) = &r.commit {
        let _ = writeln!(t, "  Analysed commit: {short}");
    }
    let _ = writeln!(t, "  Generator:       {}", r.generator);
    if !r.model.is_empty() {
        let _ = writeln!(t, "  Model:           {}", r.model);
    }
    for (label, value) in &r.inputs {
        let _ = writeln!(t, "  {:<16} {value}", format!("{label}:"));
    }
    let _ = writeln!(t, "  Requested by:    {}", r.requested_by);
    if let Some(s) = when(r.started) {
        let _ = writeln!(t, "  Started:         {s}");
    }
    if let Some(s) = when(r.finished) {
        let _ = writeln!(t, "  Completed:       {s}");
    }
    if let Some(d) = r.duration() {
        let _ = writeln!(t, "  Duration:        {d}");
    }
    let _ = writeln!(t, "  Job:             {}", r.job_id);
    if !r.documents.is_empty() {
        let _ = writeln!(t, "\nBOUNDED CONTEXT DOCUMENTS");
        for d in &r.documents {
            let _ = writeln!(t, "\n  {}", d.title.as_deref().unwrap_or(&d.path));
            let _ = writeln!(t, "  {} ({}, {})", d.path, d.change, human_size(d.size));
            for (i, s) in d.sections.iter().enumerate() {
                let _ = writeln!(t, "    {}. {s}", i + 1);
            }
        }
        if r.more_documents > 0 {
            let _ = writeln!(t, "\n  ...and {} more document(s) in the portal.", r.more_documents);
        }
    }
    if !r.others.is_empty() {
        let _ = writeln!(t, "\n{}", if r.documents.is_empty() { "GENERATED FILES" } else { "SUPPORTING FILES" });
        for (path, change) in &r.others {
            let _ = writeln!(t, "  - {path} ({change})");
        }
        if r.more_others > 0 {
            let _ = writeln!(t, "  ...and {} more file(s) in the portal.", r.more_others);
        }
    }
    let _ = writeln!(t, "\nNEXT STEPS");
    let _ = writeln!(t, "  1. Open the job in the portal to preview each document and compare it with the repository.");
    let _ =
        writeln!(t, "  2. Check the content carefully: it was generated by an AI agent and may contain inaccuracies.");
    let _ = writeln!(t, "  3. When you are satisfied, open a pull request from the portal. Pull requests are never merged automatically.");
    let _ =
        writeln!(t, "\n--\nThis is an automated message from the AT&T Bounded Context Portal. Please do not reply.");
    let _ = writeln!(
        t,
        "(c) {} AT&T Intellectual Property. All rights reserved. AT&T Proprietary (Internal Use Only).",
        Utc::now().format("%Y")
    );
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_skips_code_fences_and_cleans_headings() {
        let md = "# Bounded Context: **Orders**\n\n## 1. Purpose\n```\n## not a heading\n```\n## 2. [Domain model](#x) `core`\n### deep\n";
        let (title, sections) = outline(md);
        assert_eq!(title.as_deref(), Some("Bounded Context: Orders"));
        assert_eq!(sections, vec!["1. Purpose", "2. Domain model core"]);
    }

    #[test]
    fn fallback_address_strips_enterprise_suffix() {
        assert_eq!(fallback_address("mz7819_ATT", "att.com").unwrap().to_string(), "mz7819@att.com");
        assert_eq!(fallback_address("octocat", "example.com").unwrap().to_string(), "octocat@example.com");
        assert!(fallback_address("_ATT", "att.com").is_none());
    }

    #[test]
    fn key_documents() {
        assert!(is_key_document("docs/BOUNDED-CONTEXT-orders.md"));
        assert!(is_key_document("svc/docs/bounded-contexts/billing.md"));
        assert!(is_key_document("docs/INFRA-CONTEXT-platform.md"));
        assert!(!is_key_document("docs/archetypes/01-overview.md"));
        assert!(!is_key_document("README.md"));
    }

    #[test]
    fn html_is_escaped() {
        assert_eq!(esc(r#"<a href="x">'&'</a>"#), "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;");
    }
}
