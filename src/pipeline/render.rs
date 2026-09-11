//! S7 — rendering the handoff artifact (spec §12).
//!
//! Four layers, cheapest first, so a receiving agent can stop reading as soon
//! as it knows enough:
//!
//! * **L0 Brief** — goal, current step, next actions, hard constraints,
//!   dead ends, verify-first commands, repository drift.
//! * **L1 Items** — every active item with provenance, plus the ledgers.
//! * **L2 Recency tail** — masked rows of the end of the session.
//! * **L3 Retrieval** — the source, and the exact `sctxx expand` commands.
//!
//! Budget enforcement lives here, in Rust: items are emitted in priority order
//! until the layer budget is reached, and anything omitted is named so the
//! reader knows it exists.

use crate::ir::Session;
use crate::pipeline::fold::ops::ItemKind;
use crate::pipeline::fold::prompt::HANDOFF_PREAMBLE;
use crate::pipeline::fold::state::{FoldState, Item, ItemStatus};
use crate::pipeline::ledgers::{ErrorStatus, Ledgers};
use crate::pipeline::mask::Row;
use crate::pipeline::reconcile::Reconciliation;
use crate::vendor::codex::secrets::{RedactMode, redact};
use crate::vendor::codex::truncate::approx_token_count;
use serde::Serialize;

/// Which layers to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layers {
    pub brief: bool,
    pub items: bool,
    pub tail: bool,
    pub retrieval: bool,
}

impl Default for Layers {
    fn default() -> Self {
        Self {
            brief: true,
            items: true,
            tail: true,
            retrieval: true,
        }
    }
}

impl Layers {
    /// Parse a `--layers L0,L1,L2,L3` value.
    pub fn parse(value: &str) -> crate::error::Result<Self> {
        let mut layers = Layers {
            brief: false,
            items: false,
            tail: false,
            retrieval: false,
        };
        for part in value
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            match part.to_ascii_uppercase().as_str() {
                "L0" => layers.brief = true,
                "L1" => layers.items = true,
                "L2" => layers.tail = true,
                "L3" => layers.retrieval = true,
                other => {
                    return Err(crate::error::Error::Usage(format!(
                        "unknown layer `{other}` (expected L0, L1, L2, or L3)"
                    )));
                }
            }
        }
        Ok(layers)
    }
}

/// Rendering options.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Total artifact budget in tokens (the tail is counted separately).
    pub budget: usize,
    pub layers: Layers,
    pub mode: &'static str,
    pub llm: String,
    pub redact: RedactMode,
    /// Tokens the masked view of the whole session costs. Known only to the
    /// pipeline, which is why it is passed in rather than recomputed here.
    pub masked_tokens: usize,
    /// Tokens this artifact costs. `Extraction::markdown` fills it from a first
    /// pass, so the header can state its own size.
    pub artifact_tokens: usize,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            budget: 8_000,
            layers: Layers::default(),
            mode: "standard",
            llm: "none".to_string(),
            redact: RedactMode::Default,
            masked_tokens: 0,
            artifact_tokens: 0,
        }
    }
}

/// Everything the renderer needs.
#[derive(Debug)]
pub struct Artifact<'a> {
    pub session: &'a Session,
    pub ledgers: &'a Ledgers,
    pub state: &'a FoldState,
    pub reconciliation: &'a Reconciliation,
    pub tail: &'a [Row],
    pub options: &'a RenderOptions,
}

/// Token accounting for `report.json`.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct TokenCounts {
    pub raw: usize,
    pub masked: usize,
    pub artifact: usize,
    pub tail: usize,
}

/// The L0 budget: a brief that does not fit on a screen is not a brief.
const BRIEF_BUDGET: usize = 1_200;

/// Render `handoff.md`.
pub fn markdown(artifact: &Artifact<'_>) -> String {
    let mut out = String::new();
    out.push_str(&front_matter(artifact));
    out.push_str(&format!("\n# Handoff: {}\n\n", title(artifact)));
    out.push_str(preamble());

    let mut remaining = artifact.options.budget;
    if artifact.options.layers.brief {
        let brief = render_brief(artifact);
        remaining = remaining.saturating_sub(approx_token_count(&brief));
        out.push_str(&brief);
    }
    if artifact.options.layers.items {
        out.push_str(&render_items(artifact, remaining));
    }
    if artifact.options.layers.tail {
        out.push_str(&render_tail(artifact));
    }
    if artifact.options.layers.retrieval {
        out.push_str(&render_retrieval(artifact));
    }
    redact(&out, artifact.options.redact)
}

fn preamble() -> &'static str {
    // The preamble template's front matter is not part of the artifact.
    let body = HANDOFF_PREAMBLE;
    let after_comment = match body.find("-->") {
        Some(end) => &body[end + 3..],
        None => body,
    };
    let trimmed = after_comment.trim_start();
    match trimmed
        .strip_prefix("---")
        .and_then(|rest| rest.find("\n---").map(|end| &rest[end + 4..]))
    {
        Some(rest) => rest.trim_start(),
        None => trimmed,
    }
}

fn title(artifact: &Artifact<'_>) -> String {
    if let Some(title) = &artifact.session.meta.title {
        return title.clone();
    }
    if let Some(goal) = artifact.state.active_of(ItemKind::Goal).first() {
        return goal.text.clone();
    }
    if let Some(first) = artifact.ledgers.user_messages.first() {
        let words: Vec<&str> = first.text.split_whitespace().take(12).collect();
        return words.join(" ");
    }
    format!(
        "{} session {}",
        artifact.session.agent.label(),
        artifact.session.id
    )
}

fn front_matter(artifact: &Artifact<'_>) -> String {
    let session = artifact.session;
    let meta = &session.meta;
    let reconciliation = artifact.reconciliation;
    let counts = token_counts(artifact);
    let prompts: Vec<String> = crate::pipeline::fold::prompt::manifest()
        .into_iter()
        .map(|(id, version)| format!("{id}: {version}"))
        .collect();

    let mut out = String::from("---\n");
    out.push_str("schema: sctxx.handoff/v1\n");
    out.push_str(&format!("sctxx: {}\n", crate::VERSION));
    out.push_str(&format!(
        "source: {{agent: {}, session: {}, events: {}, active: {}, user_turns: {}",
        session.agent.slug(),
        session.id,
        session.events.len(),
        session.active.len(),
        session.user_turns()
    ));
    if let Some(started) = &meta.started_at {
        out.push_str(&format!(", started: {started}"));
    }
    if let Some(ended) = &meta.ended_at {
        out.push_str(&format!(", ended: {ended}"));
    }
    if let Some(cwd) = meta.cwd.as_deref() {
        out.push_str(&format!(", cwd: {}", posix(cwd)));
    }
    if let Some(branch) = &meta.git_branch {
        out.push_str(&format!(", branch: {branch}"));
    }
    out.push_str("}\n");
    out.push_str(&format!("mode: {}\n", artifact.options.mode));
    out.push_str(&format!("llm: {}\n", artifact.options.llm));
    out.push_str(&format!("prompts: {{{}}}\n", prompts.join(", ")));
    out.push_str(&format!(
        "verification: {{repo: {}, head: {}, commits_since_session: {}, stale: {}, contradicted: {}}}\n",
        reconciliation.repo.as_deref().map(posix).unwrap_or_else(|| "none".to_string()),
        reconciliation.head.as_deref().map(short_sha).unwrap_or_else(|| "none".to_string()),
        reconciliation.commits_since_session.len(),
        reconciliation.stale(),
        reconciliation.contradictions()
    ));
    out.push_str(&format!(
        "tokens: {{raw: {}, masked: {}, artifact: {}, tail: {}}}\n",
        counts.raw, counts.masked, counts.artifact, counts.tail
    ));
    out.push_str("---\n");
    out
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

fn posix(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// L0: what the next agent must know before touching anything.
fn render_brief(artifact: &Artifact<'_>) -> String {
    let state = artifact.state;
    let ledgers = artifact.ledgers;
    let mut out = String::from("\n## L0 · Brief\n\n");
    let mut budget = BRIEF_BUDGET;

    let push = |out: &mut String, text: String, budget: &mut usize| {
        let cost = approx_token_count(&text);
        if cost > *budget {
            return;
        }
        *budget -= cost;
        out.push_str(&text);
    };

    match state.active_of(ItemKind::Goal).first() {
        Some(goal) => push(
            &mut out,
            format!(
                "**Goal** ({}): {} {}\n\n",
                goal.id,
                goal.text,
                goal.provenance()
            ),
            &mut budget,
        ),
        // Deterministic mode has no fold, so the first human message is the
        // most honest statement of intent available.
        None => {
            if let Some(first) = ledgers.user_messages.first() {
                push(
                    &mut out,
                    format!(
                        "**Goal** (from the first user message, not model-inferred): {} [evt {}]\n\n",
                        one_line(&first.text, 300),
                        first.evt
                    ),
                    &mut budget,
                );
            }
        }
    }

    // The last thing the human asked is the sharpest statement of intent the
    // transcript contains, and it costs nothing to compute.
    if ledgers.user_messages.len() > 1
        && let Some(last) = ledgers.user_messages.last()
    {
        push(
            &mut out,
            format!(
                "**Last user request**: {} [evt {}]\n\n",
                one_line(&last.text, 300),
                last.evt
            ),
            &mut budget,
        );
    }

    if let Some(step) = state.active_of(ItemKind::CurrentStep).first() {
        push(
            &mut out,
            format!(
                "**Current step** ({}): {} {}\n\n",
                step.id,
                step.text,
                step.provenance()
            ),
            &mut budget,
        );
    }

    let next_actions = state.active_of(ItemKind::NextAction);
    if !next_actions.is_empty() {
        let mut block = String::from("**Next actions**\n");
        for (index, item) in next_actions.iter().enumerate() {
            block.push_str(&format!(
                "{}. ({}) {} {}\n",
                index + 1,
                item.id,
                item.text,
                item.provenance()
            ));
        }
        block.push('\n');
        push(&mut out, block, &mut budget);
    } else if let Some(block) = ledger_next_actions(ledgers) {
        // No fold ran, so derive next actions from the ledgers and label them
        // as derived. A deterministic artifact still has to be actionable.
        push(&mut out, block, &mut budget);
    }

    let constraints = state.active_of(ItemKind::Constraint);
    if !constraints.is_empty() {
        let mut block = String::from("**Hard constraints** (binding user instructions)\n");
        for item in &constraints {
            let quote = item.quote.as_deref().unwrap_or(&item.text);
            block.push_str(&format!(
                "- ({}) \"{}\" {}\n",
                item.id,
                quote,
                item.provenance()
            ));
        }
        block.push('\n');
        push(&mut out, block, &mut budget);
    }

    let dead_ends = state.active_of(ItemKind::DeadEnd);
    if !dead_ends.is_empty() {
        let mut block = String::from("**Don't retry**\n");
        for item in dead_ends.iter().take(5) {
            let why = item
                .why
                .as_deref()
                .map(|why| format!(" — {why}"))
                .unwrap_or_default();
            block.push_str(&format!(
                "- ({}) {}{} {}\n",
                item.id,
                item.text,
                why,
                item.provenance()
            ));
        }
        block.push('\n');
        push(&mut out, block, &mut budget);
    }

    // Verify first: the commands that tell the next agent where reality is.
    let mut verify: Vec<String> = vec!["git status".into(), "git log --oneline -5".into()];
    for command in ledgers.last_command_status().iter().take(3) {
        if matches!(
            command.category,
            crate::pipeline::ledgers::CmdCategory::Test
                | crate::pipeline::ledgers::CmdCategory::Build
                | crate::pipeline::ledgers::CmdCategory::Lint
        ) {
            verify.push(command.normalized.clone());
        }
    }
    verify.dedup();
    push(
        &mut out,
        format!(
            "**Verify first**\n{}\n",
            verify
                .iter()
                .map(|command| format!("- `{command}`\n"))
                .collect::<String>()
        ),
        &mut budget,
    );

    if artifact.reconciliation.repo_moved() {
        let mut block = String::from("**Since this session**\n");
        for commit in artifact
            .reconciliation
            .commits_since_session
            .iter()
            .take(10)
        {
            block.push_str(&format!("- new commit: {commit}\n"));
        }
        for file in artifact
            .reconciliation
            .changed_since_session
            .iter()
            .take(10)
        {
            block.push_str(&format!("- `{file}` changed after the session ended\n"));
        }
        block.push('\n');
        push(&mut out, block, &mut budget);
    }
    if artifact.reconciliation.contradictions() > 0 {
        push(
            &mut out,
            format!(
                "> [!WARNING]\n> {} claim(s) in this artifact contradict the current repository: {}.\n\n",
                artifact.reconciliation.contradictions(),
                artifact.reconciliation.contradicted_files.join(", ")
            ),
            &mut budget,
        );
    }
    out
}

/// Next actions a reviewer could read straight off the ledgers: the plan item
/// that was in progress, and any command whose last run failed.
fn ledger_next_actions(ledgers: &Ledgers) -> Option<String> {
    let mut actions: Vec<String> = Vec::new();

    if let Some(plan) = &ledgers.plan {
        for item in plan
            .items
            .iter()
            .filter(|item| matches!(item.status.as_str(), "in_progress" | "active" | "pending"))
        {
            actions.push(format!(
                "{} (plan item, {}) [evt {}]",
                item.text, item.status, plan.evt
            ));
        }
    }
    for command in ledgers
        .last_command_status()
        .iter()
        .filter(|command| command.failed())
    {
        actions.push(format!(
            "re-run `{}` \u{2014} its last run FAILED [evt {}]",
            command.normalized, command.evt
        ));
    }
    for error in ledgers.unresolved_errors().iter().take(2) {
        actions.push(format!(
            "resolve `{}` (\u{d7}{}) \u{2014} {} [evt {}\u{2013}{}]",
            error.sig,
            error.occurrences,
            one_line(&error.example, 140),
            error.first_evt,
            error.last_evt
        ));
    }
    if actions.is_empty() {
        return None;
    }

    let mut block =
        String::from("**Next actions** (derived from the ledgers, not model-inferred)\n");
    for (index, action) in actions.iter().take(4).enumerate() {
        block.push_str(&format!("{}. {action}\n", index + 1));
    }
    block.push('\n');
    Some(block)
}

/// L1: every active item with provenance, then the ledgers.
fn render_items(artifact: &Artifact<'_>, budget: usize) -> String {
    let mut out = String::from("\n## L1 · Items\n");
    let mut remaining = budget;
    let mut omitted: Vec<&str> = Vec::new();

    for kind in ItemKind::PRIORITY {
        let items = artifact.state.active_of(kind);
        if items.is_empty() {
            continue;
        }
        let mut block = format!("\n### {}\n", kind.heading());
        for item in items {
            let line = item_line(item);
            let cost = approx_token_count(&line);
            if cost > remaining {
                omitted.push(&item.id);
                continue;
            }
            remaining -= cost;
            block.push_str(&line);
        }
        out.push_str(&block);
    }

    // Only reversals a reader must know about: a superseded next action is
    // just bookkeeping, a superseded decision is a warning.
    let superseded: Vec<&Item> = artifact
        .state
        .superseded()
        .into_iter()
        .filter(|item| {
            matches!(
                item.kind,
                ItemKind::Decision | ItemKind::Constraint | ItemKind::Goal
            )
        })
        .collect();
    if !superseded.is_empty() {
        out.push_str("\n### Earlier decisions later reversed\n");
        for item in &superseded {
            let by = match &item.status {
                ItemStatus::Superseded { by } => by.as_str(),
                _ => "?",
            };
            out.push_str(&format!(
                "- ~~{}~~ {} — superseded by {by} {}\n",
                item.id,
                item.text,
                item.provenance()
            ));
        }
    }

    out.push_str(&render_ledgers(artifact));

    if !omitted.is_empty() {
        out.push_str(&format!(
            "\n_{} item(s) omitted for the artifact budget: {}. See `state.json`._\n",
            omitted.len(),
            omitted.join(", ")
        ));
    }
    out
}

fn item_line(item: &Item) -> String {
    let mut line = format!("- {} · {}", item.id, item.confidence.label());
    if item.verified != crate::pipeline::fold::state::Verification::Unchecked {
        line.push_str(&format!(" · {}", item.verified.label()));
    }
    line.push_str(&format!(" — {}", item.text));
    if let Some(quote) = &item.quote {
        line.push_str(&format!(" · quote: \"{quote}\""));
    }
    if let Some(why) = &item.why {
        line.push_str(&format!(" · *why*: {why}"));
    }
    if !item.rejected.is_empty() {
        line.push_str(&format!(" · *rejected*: {}", item.rejected.join(", ")));
    }
    line.push_str(&format!(" {}\n", item.provenance()));
    line
}

/// The deterministic ledgers: true whether or not a model ran.
fn render_ledgers(artifact: &Artifact<'_>) -> String {
    let ledgers = artifact.ledgers;
    let mut out = String::new();

    let edited = ledgers.edited_files();
    if !edited.is_empty() {
        out.push_str(
            "\n### Files touched\n\n| path | ops | last evt | status |\n|---|---|---|---|\n",
        );
        for file in edited.iter().take(40) {
            let mut status = if file.deleted { "deleted" } else { "exists" }.to_string();
            if artifact.reconciliation.missing_files.contains(&file.path) {
                status = "missing now".to_string();
            } else if artifact
                .reconciliation
                .changed_since_session
                .contains(&file.path)
            {
                status = "changed since session".to_string();
            } else if artifact
                .reconciliation
                .contradicted_files
                .contains(&file.path)
            {
                status = "still exists (contradicts)".to_string();
            }
            if file.inferred {
                status.push_str(" · inferred");
            }
            out.push_str(&format!(
                "| `{}` | edit×{} read×{} | {} | {} |\n",
                file.path, file.edits, file.reads, file.last_evt, status
            ));
        }
        if edited.len() > 40 {
            out.push_str(&format!(
                "\n_{} more files in `ledgers.json`._\n",
                edited.len() - 40
            ));
        }
    }

    let statuses = ledgers.last_command_status();
    if !statuses.is_empty() {
        out.push_str("\n### Last known command status\n");
        for command in statuses.iter().take(12) {
            out.push_str(&format!(
                "- `{}` → **{}**{} [evt {}]\n",
                command.normalized,
                command.status(),
                command
                    .exit_code
                    .map(|code| format!(" (exit {code})"))
                    .unwrap_or_default(),
                command.evt
            ));
        }
    }

    let unresolved = ledgers.unresolved_errors();
    if !unresolved.is_empty() {
        out.push_str("\n### Unresolved errors\n");
        for error in unresolved.iter().take(10) {
            out.push_str(&format!(
                "- `{}` ×{} — {} [evt {}–{}]\n",
                error.sig,
                error.occurrences,
                one_line(&error.example, 200),
                error.first_evt,
                error.last_evt
            ));
        }
    }

    let resolved = ledgers
        .errors
        .iter()
        .filter(|error| matches!(error.status, ErrorStatus::Resolved { .. }))
        .count();
    if resolved > 0 {
        out.push_str(&format!(
            "\n_{resolved} earlier error signature(s) were resolved._\n"
        ));
    }

    if let Some(plan) = &ledgers.plan {
        out.push_str(&format!(
            "\n### Plan as last published [evt {}]\n",
            plan.evt
        ));
        for item in &plan.items {
            let mark = match item.status.as_str() {
                "completed" | "done" => "x",
                _ => " ",
            };
            out.push_str(&format!("- [{mark}] {} ({})\n", item.text, item.status));
        }
    }

    if !ledgers.git.commits.is_empty() || !ledgers.git.pull_requests.is_empty() {
        out.push_str("\n### Git\n");
        for commit in ledgers.git.commits.iter().take(15) {
            let missing = artifact
                .reconciliation
                .missing_commits
                .contains(&commit.sha);
            out.push_str(&format!(
                "- {} {}{}\n",
                commit.sha,
                commit.subject,
                if missing {
                    " — no longer in this repository (rebased or squashed?)"
                } else {
                    ""
                }
            ));
        }
        for url in &ledgers.git.pull_requests {
            out.push_str(&format!("- pull request: {url}\n"));
        }
        if ledgers.git.pushed {
            out.push_str("- the session pushed at least once\n");
        }
    }

    if !ledgers.prior_summaries.is_empty() {
        out.push_str(&format!(
            "\n_This session contained {} provider compaction summary/summaries; they are low-trust \
             and were used only as seeds._\n",
            ledgers.prior_summaries.len()
        ));
    }
    out
}

/// L2: the end of the session, near-verbatim.
fn render_tail(artifact: &Artifact<'_>) -> String {
    if artifact.tail.is_empty() {
        return String::new();
    }
    let first = artifact.tail.first().map(|row| row.evt).unwrap_or(0);
    let last = artifact.tail.last().map(|row| row.evt).unwrap_or(0);

    let body: String = artifact
        .tail
        .iter()
        .map(|row| format!("{}  (evt {})\n", row.text, row.evt))
        .collect();

    // Transcript text routinely contains code fences and markdown headings.
    // A fixed ```-fence would let that content escape the block and become
    // artifact structure, so the fence is always longer than anything inside.
    let fence = fence_for(&body);
    format!(
        "\n## L2 · Recent activity (masked, evt {first}–{last})\n\n{fence}text\n{body}{fence}\n"
    )
}

/// A backtick fence guaranteed to be longer than any run of backticks in
/// `content`, so the content cannot terminate it early.
fn fence_for(content: &str) -> String {
    let mut longest = 0usize;
    let mut current = 0usize;
    for character in content.chars() {
        if character == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    "`".repeat(longest.max(2) + 1)
}

/// L3: how to get back to the raw events.
fn render_retrieval(artifact: &Artifact<'_>) -> String {
    let session = artifact.session;
    let reference = format!("{}:{}", session.agent.slug(), session.id);
    let mut out = String::from("\n## L3 · Retrieval\n\n");
    out.push_str(&format!(
        "Source: {} session `{}`\n",
        session.agent.label(),
        session.id
    ));
    for path in &session.source_paths {
        out.push_str(&format!("- `{}`\n", posix(path)));
    }
    out.push_str(&format!(
        "\nExpand any pointer:\n```sh\nsctxx expand {reference} <a>..<b> --context 3\n"
    ));

    // Concrete commands for the items most likely to need detail.
    let mut examples: Vec<String> = Vec::new();
    for kind in [
        ItemKind::CurrentStep,
        ItemKind::NextAction,
        ItemKind::DeadEnd,
    ] {
        for item in artifact.state.active_of(kind) {
            if let Some(span) = item.span() {
                examples.push(format!(
                    "sctxx expand {reference} {}..{}   # {}\n",
                    span.start, span.end, item.id
                ));
            }
        }
    }
    for example in examples.iter().take(4) {
        out.push_str(example);
    }
    out.push_str("```\n");
    out.push_str(&format!(
        "\nFull state including superseded and dropped items: `state.json`. \
         All deterministic ledgers: `ledgers.json`. Run details: `report.json`.\n\
         \nSession source hash: `{}`\n",
        short_sha(&session.source_hash)
    ));
    if !session.diagnostics.is_empty() {
        out.push_str(&format!(
            "\n_{} adapter diagnostic(s) recorded; see `report.json`._\n",
            session.diagnostics.len()
        ));
    }
    out
}

/// Token accounting for the artifact header and `report.json`.
///
/// `masked` and `artifact` come from the caller: the renderer cannot compute
/// either. The masked count lives in the pipeline (it is the sum over every
/// row, and the artifact only carries the tail), and the artifact count is the
/// size of the text being rendered, which is why `Extraction::markdown`
/// measures a first pass and renders again with the answer. They used to be
/// hardcoded to 0, which made every artifact's header claim the session had no
/// masked tokens at all.
pub fn token_counts(artifact: &Artifact<'_>) -> TokenCounts {
    let raw: usize = artifact
        .session
        .source_paths
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|meta| meta.len() as usize / 4)
        .sum();
    TokenCounts {
        raw,
        masked: artifact.options.masked_tokens,
        artifact: artifact.options.artifact_tokens,
        tail: artifact.tail.iter().map(|row| row.tokens).sum(),
    }
}

fn one_line(text: &str, max_bytes: usize) -> String {
    let single = text.replace(['\n', '\r'], " ");
    crate::vendor::codex::truncate::truncate_middle_bytes(single.trim(), max_bytes)
}

/// The machine-readable artifact (`handoff.json`, schema `sctxx.handoff/v1`).
#[derive(Debug, Serialize)]
pub struct HandoffJson<'a> {
    pub schema: &'static str,
    pub sctxx: &'static str,
    pub session: SessionJson<'a>,
    pub mode: &'static str,
    pub llm: &'a str,
    pub prompts: Vec<(String, u32)>,
    pub items: Vec<&'a Item>,
    pub ledgers: &'a Ledgers,
    pub verification: &'a Reconciliation,
    pub tail: Vec<TailRowJson<'a>>,
}

/// Session identity, as it appears in `handoff.json`.
#[derive(Debug, Serialize)]
pub struct SessionJson<'a> {
    pub agent: &'static str,
    pub id: &'a str,
    pub source_paths: Vec<String>,
    pub source_hash: &'a str,
    pub events: usize,
    pub active: usize,
    pub user_turns: usize,
    #[serde(flatten)]
    pub meta: &'a crate::ir::SessionMeta,
}

/// One tail row in `handoff.json`.
#[derive(Debug, Serialize)]
pub struct TailRowJson<'a> {
    pub evt: u32,
    pub text: &'a str,
}

/// Render `handoff.json`.
pub fn json<'a>(artifact: &'a Artifact<'a>) -> HandoffJson<'a> {
    HandoffJson {
        schema: "sctxx.handoff/v1",
        sctxx: crate::VERSION,
        session: SessionJson {
            agent: artifact.session.agent.slug(),
            id: &artifact.session.id,
            source_paths: artifact
                .session
                .source_paths
                .iter()
                .map(|p| posix(p))
                .collect(),
            source_hash: &artifact.session.source_hash,
            events: artifact.session.events.len(),
            active: artifact.session.active.len(),
            user_turns: artifact.session.user_turns(),
            meta: &artifact.session.meta,
        },
        mode: artifact.options.mode,
        llm: &artifact.options.llm,
        prompts: crate::pipeline::fold::prompt::manifest(),
        items: artifact.state.active(),
        ledgers: artifact.ledgers,
        verification: artifact.reconciliation,
        tail: artifact
            .tail
            .iter()
            .map(|row| TailRowJson {
                evt: row.evt,
                text: &row.text,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{AgentKind, SessionMeta};
    use crate::pipeline::fold::ops::{Confidence, EvtRange, NewItem, Op};
    use crate::vendor::codex::tiered_input::Tier;

    fn session() -> Session {
        Session {
            agent: AgentKind::ClaudeCode,
            id: "7c1e8f82".into(),
            source_paths: vec![std::path::PathBuf::from(
                "/home/a/.claude/projects/p/7c1e8f82.jsonl",
            )],
            source_hash: "abcdef0123456789".into(),
            meta: SessionMeta {
                cwd: Some(std::path::PathBuf::from("/home/a/repo")),
                git_branch: Some("feat/x".into()),
                ..SessionMeta::default()
            },
            events: vec![],
            active: vec![],
            native_compactions: vec![],
            diagnostics: vec![],
        }
    }

    fn new_item(kind: ItemKind, text: &str) -> NewItem {
        NewItem {
            kind,
            text: text.into(),
            why: None,
            quote: None,
            rejected: vec![],
            sources: vec![EvtRange::new(10, 20)],
            confidence: Confidence::High,
        }
    }

    fn artifact_with(state: &FoldState, ledgers: &Ledgers, tail: &[Row]) -> String {
        let session = session();
        let reconciliation = Reconciliation::default();
        let options = RenderOptions::default();
        markdown(&Artifact {
            session: &session,
            ledgers,
            state,
            reconciliation: &reconciliation,
            tail,
            options: &options,
        })
    }

    #[test]
    fn the_artifact_has_front_matter_a_preamble_and_every_layer() {
        let mut state = FoldState::new();
        state.apply(
            "c0",
            &Op::Add {
                item: new_item(ItemKind::Goal, "ship the migration"),
            },
        );
        let tail = vec![Row {
            evt: 99,
            tier: Tier::User,
            text: "[user] carry on".into(),
            tokens: 4,
            is_human_turn: true,
        }];
        let out = artifact_with(&state, &Ledgers::default(), &tail);
        assert!(out.starts_with("---\nschema: sctxx.handoff/v1"), "{out}");
        assert!(
            out.contains("A different coding agent worked on this task"),
            "{out}"
        );
        for heading in [
            "## L0 · Brief",
            "## L1 · Items",
            "## L2 · Recent activity",
            "## L3 · Retrieval",
        ] {
            assert!(out.contains(heading), "missing {heading}");
        }
        assert!(out.contains("[evt 10–20]"), "provenance missing: {out}");
    }

    #[test]
    fn a_constraint_is_quoted_in_the_brief() {
        let mut state = FoldState::new();
        let mut constraint = new_item(ItemKind::Constraint, "never push to main");
        constraint.quote = Some("never push to main without asking".into());
        state.apply("c0", &Op::Add { item: constraint });
        let out = artifact_with(&state, &Ledgers::default(), &[]);
        assert!(out.contains("Hard constraints"), "{out}");
        assert!(
            out.contains("\"never push to main without asking\""),
            "{out}"
        );
    }

    #[test]
    fn without_a_fold_the_goal_comes_from_the_first_user_message_and_says_so() {
        let mut ledgers = Ledgers::default();
        ledgers
            .user_messages
            .push(crate::pipeline::ledgers::UserMessageRecord {
                evt: 3,
                text: "implement the manifest loader with trust tiers".into(),
            });
        let out = artifact_with(&FoldState::new(), &ledgers, &[]);
        assert!(out.contains("not model-inferred"), "{out}");
        assert!(out.contains("manifest loader"), "{out}");
    }

    #[test]
    fn without_a_fold_the_next_actions_come_from_the_ledgers() {
        let mut ledgers = Ledgers::default();
        ledgers
            .commands
            .push(crate::pipeline::ledgers::CommandRecord {
                evt: 14,
                command: "pnpm test".into(),
                normalized: "pnpm test".into(),
                exit_code: Some(1),
                is_error: Some(true),
                category: crate::pipeline::ledgers::CmdCategory::Test,
                ..Default::default()
            });
        ledgers.plan = Some(crate::pipeline::ledgers::PlanLedger {
            evt: 5,
            items: vec![crate::ir::PlanItem {
                text: "wire tier 3".into(),
                status: "in_progress".into(),
            }],
        });
        let out = artifact_with(&FoldState::new(), &ledgers, &[]);
        assert!(
            out.contains("**Next actions** (derived from the ledgers"),
            "{out}"
        );
        assert!(out.contains("wire tier 3"), "{out}");
        assert!(out.contains("re-run `pnpm test`"), "{out}");
    }

    #[test]
    fn folded_next_actions_replace_the_ledger_derived_ones() {
        let mut state = FoldState::new();
        state.apply(
            "c0",
            &Op::Add {
                item: new_item(ItemKind::NextAction, "fix spawn()"),
            },
        );
        let mut ledgers = Ledgers::default();
        ledgers.plan = Some(crate::pipeline::ledgers::PlanLedger {
            evt: 5,
            items: vec![crate::ir::PlanItem {
                text: "wire tier 3".into(),
                status: "in_progress".into(),
            }],
        });
        let out = artifact_with(&state, &ledgers, &[]);
        assert!(out.contains("fix spawn()"), "{out}");
        assert!(!out.contains("derived from the ledgers"), "{out}");
    }

    #[test]
    fn the_last_user_request_is_surfaced_when_there_was_more_than_one() {
        let mut ledgers = Ledgers::default();
        for (evt, text) in [
            (0u32, "build the loader"),
            (12, "now make tier 3 sandboxed"),
        ] {
            ledgers
                .user_messages
                .push(crate::pipeline::ledgers::UserMessageRecord {
                    evt,
                    text: text.into(),
                });
        }
        let out = artifact_with(&FoldState::new(), &ledgers, &[]);
        assert!(
            out.contains("**Last user request**: now make tier 3 sandboxed"),
            "{out}"
        );
    }

    #[test]
    fn every_artifact_tells_the_next_agent_how_to_verify_and_how_to_expand() {
        let out = artifact_with(&FoldState::new(), &Ledgers::default(), &[]);
        assert!(out.contains("**Verify first**"), "{out}");
        assert!(out.contains("git status"), "{out}");
        assert!(out.contains("sctxx expand claude:7c1e8f82"), "{out}");
    }

    #[test]
    fn items_over_the_budget_are_named_rather_than_silently_dropped() {
        let mut state = FoldState::new();
        for i in 0..40 {
            state.apply(
                "c0",
                &Op::Add {
                    item: new_item(ItemKind::EnvFact, &format!("fact number {i} ").repeat(6)),
                },
            );
        }
        let session = session();
        let reconciliation = Reconciliation::default();
        let options = RenderOptions {
            budget: 300,
            ..RenderOptions::default()
        };
        let ledgers = Ledgers::default();
        let out = markdown(&Artifact {
            session: &session,
            ledgers: &ledgers,
            state: &state,
            reconciliation: &reconciliation,
            tail: &[],
            options: &options,
        });
        assert!(out.contains("omitted for the artifact budget"), "{out}");
    }

    #[test]
    fn a_contradiction_is_flagged_prominently() {
        let session = session();
        let reconciliation = Reconciliation {
            contradicted_files: vec!["src/gone.rs".into()],
            ..Reconciliation::default()
        };
        let options = RenderOptions::default();
        let ledgers = Ledgers::default();
        let out = markdown(&Artifact {
            session: &session,
            ledgers: &ledgers,
            state: &FoldState::new(),
            reconciliation: &reconciliation,
            tail: &[],
            options: &options,
        });
        assert!(out.contains("[!WARNING]"), "{out}");
        assert!(out.contains("src/gone.rs"), "{out}");
    }

    #[test]
    fn transcript_code_fences_cannot_escape_the_recency_tail() {
        // An assistant message containing a fenced block and a heading: both
        // must stay inside L2 instead of becoming artifact structure.
        let tail = vec![Row {
            evt: 99,
            tier: Tier::AssistantFinal,
            text: "[assistant] here:\n```sh\nnpm run dev\n```\n## What's now hot".into(),
            tokens: 20,
            is_human_turn: false,
        }];
        let out = artifact_with(&FoldState::new(), &Ledgers::default(), &tail);

        // Inside a fence, `## ...` is literal text, so the property to check
        // is that no transcript line escapes the fence and becomes structure.
        let mut inside_fence = false;
        let mut fence_marker = String::new();
        for line in out.lines() {
            if !inside_fence && line.starts_with("``") {
                inside_fence = true;
                // The marker is the leading backtick run; the rest is a
                // language tag (```text, ```sh).
                fence_marker = line.chars().take_while(|c| *c == '`').collect();
                continue;
            }
            if inside_fence {
                if line == fence_marker {
                    inside_fence = false;
                }
                continue;
            }
            assert!(
                !line.starts_with("## ")
                    || line.starts_with("## L0")
                    || line.starts_with("## L1")
                    || line.starts_with("## L2")
                    || line.starts_with("## L3"),
                "transcript content escaped the fence and became a heading: {line}"
            );
        }
        assert!(
            !inside_fence,
            "the recency tail fence was never closed:\n{out}"
        );
        assert!(
            out.contains("What's now hot"),
            "the tail content was lost:\n{out}"
        );
    }

    #[test]
    fn the_fence_is_always_longer_than_the_content_it_wraps() {
        assert_eq!(fence_for("no backticks"), "```");
        assert_eq!(fence_for("a ``` fence"), "````");
        assert_eq!(fence_for("a ````` fence"), "``````");
    }

    #[test]
    fn layers_can_be_selected() {
        let layers = Layers::parse("L0,L3").expect("parse");
        assert!(layers.brief && layers.retrieval);
        assert!(!layers.items && !layers.tail);
        assert_eq!(Layers::parse("L9").expect_err("reject").exit_code(), 2);
    }

    #[test]
    fn the_json_artifact_carries_the_schema_and_the_same_items() {
        let mut state = FoldState::new();
        state.apply(
            "c0",
            &Op::Add {
                item: new_item(ItemKind::Goal, "ship"),
            },
        );
        let session = session();
        let reconciliation = Reconciliation::default();
        let options = RenderOptions::default();
        let ledgers = Ledgers::default();
        let artifact = Artifact {
            session: &session,
            ledgers: &ledgers,
            state: &state,
            reconciliation: &reconciliation,
            tail: &[],
            options: &options,
        };
        let value = serde_json::to_value(json(&artifact)).expect("serialize");
        assert_eq!(value["schema"], "sctxx.handoff/v1");
        assert_eq!(value["items"][0]["id"], "G1");
        assert_eq!(value["session"]["agent"], "claude");
    }
}
