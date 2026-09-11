//! `sctxx extract` — the main command (spec §3.4).

use super::{GlobalArgs, out, out_json};
use crate::adapters::discovery;
use crate::error::{Error, Result};
use crate::llm::Selection;
use crate::pipeline::{self, ExtractOptions, Mode, render};
use crate::vendor::codex::secrets::RedactMode;
use clap::Args;
use std::io::Write;
use std::path::PathBuf;

/// `sctxx extract`
#[derive(Debug, Args)]
pub struct ExtractArgs {
    /// Session reference: `[claude|codex|pi:]<id|prefix|last[:N]>` or a path.
    reference: String,

    /// How much of the pipeline to run: fast, standard, or full.
    #[arg(long, default_value = "standard", value_parser = ["fast", "standard", "full"])]
    mode: String,

    /// Backend: none, auto, cli:<agent>, api:<provider>[/<model>], or mock.
    ///
    /// `none` is the default because the deterministic artifact is complete —
    /// every pointer, every ledger, the recency tail — and the fold costs real
    /// tokens: on a 103k-event session, 41 fold calls and 40 premap calls, about
    /// 813,000 input tokens. A tool whose promise is verified compaction should
    /// not spend that unless asked to (ADR 0007).
    #[arg(long, default_value = "none")]
    llm: String,

    /// Artifact token budget, excluding the recency tail.
    #[arg(long, default_value_t = 8_000)]
    budget: usize,

    /// The fold model's context window, in tokens. Given it, every prompt is
    /// checked against it before it is sent, and a session that cannot fit fails
    /// once, immediately, instead of once per chunk.
    #[arg(long)]
    model_context: Option<usize>,

    /// Tokens reserved for the fold model's answer.
    #[arg(long, default_value_t = 4_000)]
    max_completion: usize,

    /// Recency tail token budget.
    #[arg(long, default_value_t = 12_000)]
    tail: usize,

    /// Tokens per fold chunk.
    #[arg(long, default_value_t = 24_000)]
    chunk_tokens: usize,

    /// What the next agent wants to do, to bias extraction.
    #[arg(long, value_name = "TEXT")]
    focus: Option<String>,

    /// Repository to reconcile against (default: the session's cwd, else `.`).
    #[arg(long, value_name = "PATH")]
    repo: Option<PathBuf>,

    /// Skip repository reconciliation.
    #[arg(long)]
    no_verify: bool,

    /// Exit 7 if the repository contradicts the artifact.
    #[arg(long)]
    strict: bool,

    /// Write the artifacts here. A directory gets all five files; a path
    /// ending in .md or .json gets just that one.
    #[arg(long, value_name = "PATH")]
    out: Option<PathBuf>,

    /// Output format when writing to stdout: md, json, or both.
    #[arg(long, default_value = "md", value_parser = ["md", "json", "both"])]
    format: String,

    /// Layers to render, e.g. `L0,L1`.
    #[arg(long, default_value = "L0,L1,L2,L3")]
    layers: String,

    /// Include subagent transcripts.
    #[arg(long)]
    include_sidechains: bool,

    /// Keep readable model reasoning in masked rows.
    #[arg(long)]
    keep_reasoning: bool,

    /// Keep system and unrecognized events as rows.
    #[arg(long)]
    keep_system: bool,

    /// Redaction level: default, strict, or off (off applies to --llm none only).
    #[arg(long, default_value = "default", value_parser = ["default", "strict", "off"])]
    redact: String,

    /// Parallel premap calls.
    #[arg(long, default_value_t = 4)]
    concurrency: usize,

    /// Progress format on stderr: text or json.
    #[arg(long, default_value = "text", value_parser = ["text", "json"])]
    progress: String,

    /// Print the plan (chunks, budgets, estimated tokens) and exit.
    #[arg(long)]
    dry_run: bool,

    /// Search every project when resolving `last`.
    #[arg(long)]
    any_project: bool,

    /// Start from the newest provider compaction boundary, keeping that
    /// boundary's summary as a low-trust seed.
    #[arg(long)]
    since_compact: bool,

    /// Allow up to this fraction of lines to fail parsing, e.g. 0.05 for 5%
    /// (default 0.02). A provider version newer than sctxx can add line types
    /// it does not know yet; this is the knob for that.
    #[arg(
        long,
        default_value_t = crate::adapters::DEFAULT_MAX_BAD_LINE_RATE,
        value_name = "RATE"
    )]
    max_bad_lines: f64,
}

impl ExtractArgs {
    /// The clap definition of `sctxx extract`.
    ///
    /// The TUI's extraction form is built from *this*, not from a list of its
    /// own, which is what stops the form and the CLI from drifting apart
    /// (SC-004). Anything the form can express arrives here as argv and is
    /// parsed by clap, so the CLI stays the only authority on what is valid.
    pub fn command() -> clap::Command {
        <Self as clap::Args>::augment_args(clap::Command::new("extract"))
    }

    /// Parse an argv the way the command line would, for callers that assemble
    /// one rather than receiving it.
    pub fn parse_argv<I, T>(argv: I) -> Result<Self>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let matches = Self::command()
            .try_get_matches_from(argv)
            .map_err(|error| Error::Usage(error.to_string()))?;
        <Self as clap::FromArgMatches>::from_arg_matches(&matches)
            .map_err(|error| Error::Usage(error.to_string()))
    }

    /// The options a handoff runs with: deterministic, because the
    /// deterministic artifact already carries every `[evt a–b]` pointer, every
    /// ledger, and the recency tail — which is what a receiving agent needs —
    /// and it costs no tokens.
    ///
    /// Both callers use this: the TUI's `h`, and `sctxx handoff`. One
    /// definition, so the two cannot disagree about what a handoff costs.
    pub fn deterministic(global: &GlobalArgs, reference: &str) -> Result<ExtractOptions> {
        Self::parse_argv(["sctxx", "--llm", "none", reference])?.options(global)
    }

    /// Validate the arguments and turn them into pipeline options.
    ///
    /// Shared with `run`, so a form submission and a command line are validated
    /// by exactly the same code and cannot disagree about what is legal.
    pub fn options(&self, global: &GlobalArgs) -> Result<ExtractOptions> {
        self.build_options(global)
    }

    fn build_options(&self, global: &GlobalArgs) -> Result<ExtractOptions> {
        let mode = Mode::parse(&self.mode)?;
        let llm = Selection::parse(&self.llm)?;
        let redact = parse_redact(&self.redact, &llm, global)?;
        let layers = render::Layers::parse(&self.layers)?;
        if !(0.0..=1.0).contains(&self.max_bad_lines) {
            return Err(Error::Usage(format!(
                "--max-bad-lines must be a fraction between 0 and 1 (got {}); 0.02 tolerates 2%",
                self.max_bad_lines
            )));
        }
        Ok(ExtractOptions {
            mode,
            llm,
            budget: self.budget,
            model_context: self.model_context,
            max_completion: self.max_completion,
            tail_tokens: self.tail,
            chunk_tokens: self.chunk_tokens,
            focus: self.focus.clone(),
            repo: self.repo.clone(),
            verify: !self.no_verify,
            strict: self.strict,
            layers,
            include_sidechains: self.include_sidechains,
            since_compact: self.since_compact,
            keep_reasoning: self.keep_reasoning,
            keep_system: self.keep_system,
            redact,
            concurrency: self.concurrency,
            max_bad_line_rate: self.max_bad_lines,
        })
    }
}

pub fn run(args: &ExtractArgs, global: &GlobalArgs) -> Result<i32> {
    let options = args.options(global)?;
    let mode = options.mode;
    let llm = options.llm.clone();
    if mode == Mode::Full {
        global
            .note("note: --mode full currently behaves as standard; the probe loop is roadmap M5.");
    }

    let reference = discovery::parse_reference(&args.reference)?;
    let resolve_options = global.resolve_options(args.any_project, true);
    let summary = discovery::resolve(&reference, &resolve_options)?;

    if args.dry_run {
        return dry_run(&summary, &options, global);
    }

    // Auto-resolution can find nothing; that is a notice, not a failure.
    if llm == Selection::None {
        global.note("llm: none — producing the deterministic artifact.");
    } else if llm == Selection::Auto && crate::llm::resolve_auto().is_none() {
        global.note(&format!("note: {}", crate::llm::no_backend_hint()));
    }

    let json_progress = args.progress == "json";
    let quiet = global.quiet;
    let mut progress = |stage: &str, message: &str| {
        if quiet {
            return;
        }
        let line = if json_progress {
            serde_json::json!({ "stage": stage, "message": message }).to_string()
        } else {
            format!("[{stage}] {message}")
        };
        let _ = writeln!(std::io::stderr(), "{line}");
    };

    let extraction = pipeline::extract(&summary, &options, &mut progress)?;

    for warning in &extraction.report.warnings {
        global.note(&format!("warning: {warning}"));
    }

    match &args.out {
        Some(path) => write_output(&extraction, &options, path, global),
        None => {
            match args.format.as_str() {
                "md" => out(&extraction.markdown(&options)),
                "json" => out_json(&extraction.json(&options))?,
                "both" => {
                    out(&extraction.markdown(&options));
                    out_json(&extraction.json(&options))?;
                }
                other => {
                    return Err(Error::Usage(format!(
                        "unknown --format `{other}` (expected md, json, or both)"
                    )));
                }
            }
            Ok(0)
        }
    }
}

fn write_output(
    extraction: &pipeline::Extraction,
    options: &ExtractOptions,
    path: &std::path::Path,
    global: &GlobalArgs,
) -> Result<i32> {
    let written = pipeline::write_destination(extraction, options, path)?;
    for file in &written.paths {
        global.note(&format!("wrote {}", file.display()));
    }
    // Only a directory can be surprising to git; a named file is deliberate.
    if written.directory {
        warn_if_git_would_track(path, global);
    }
    // The path the receiving agent should read is the payload.
    out(&written.handoff.to_string_lossy());
    Ok(0)
}

/// Warn when the artifact directory sits in a repository and is not ignored.
///
/// An artifact quotes the session: user messages verbatim, file paths, error
/// output. A `git add -A` in the user's project would commit that, and push it.
/// sctxx does not edit the user's git config on its own — it says what to run.
pub fn git_track_warning(path: &std::path::Path) -> Option<String> {
    if pipeline::reconcile::is_git_ignored(path) != Some(false) {
        return None;
    }
    let shown = path.to_string_lossy();
    Some(format!(
        "warning: {shown} is not ignored by git, and it holds this session's content.\n\
         \x20        Keep it out of the repository with:\n\
         \x20          echo '{shown}/' >> \"$(git rev-parse --git-dir)/info/exclude\"\n\
         \x20        (or add it to .gitignore if you mean to commit the ignore rule)"
    ))
}

fn warn_if_git_would_track(path: &std::path::Path, global: &GlobalArgs) {
    if let Some(warning) = git_track_warning(path) {
        global.note(&warning);
    }
}

/// `--dry-run`: show the plan and the cost before spending anything.
fn dry_run(
    summary: &discovery::SessionSummary,
    options: &ExtractOptions,
    global: &GlobalArgs,
) -> Result<i32> {
    let plan_only = ExtractOptions {
        llm: Selection::None,
        verify: false,
        ..options.clone()
    };
    let extraction = pipeline::extract(summary, &plan_only, &mut |_, _| {})?;
    let plan = &extraction.plan;
    let backend = match &options.llm {
        Selection::Auto => crate::llm::resolve_auto()
            .map(|selection| selection.to_string())
            .unwrap_or_else(|| "none (nothing detected)".to_string()),
        other => other.to_string(),
    };
    // What *this* run will call, not what a fold would call. Reporting the
    // latter under `--llm none` would overstate the cost of the run being
    // planned — and this number is exactly what someone uses to decide.
    let will_fold = !matches!(options.llm, Selection::None);
    let calls = if will_fold {
        plan.chunks.len() + usize::from(!plan.tail.is_empty())
    } else {
        0
    };
    let premap = if will_fold && plan.chunks.len() > 4 {
        plan.chunks.len()
    } else {
        0
    };

    let report = serde_json::json!({
        "session": summary.reference(),
        "path": summary.path,
        "events": extraction.session.events.len(),
        "active_events": extraction.session.active.len(),
        "user_turns": extraction.session.user_turns(),
        "masked_rows": extraction.rows.len(),
        "masked_tokens": extraction.report.tokens.masked,
        "episodes": plan.episodes.len(),
        "chunks": plan.chunks.len(),
        "chunk_tokens": plan.chunk_tokens(),
        "tail_rows": plan.tail.len(),
        "tail_tokens": extraction.report.tokens.tail,
        "llm": backend,
        "planned_fold_calls": calls,
        "planned_premap_calls": premap,
        "estimated_prompt_tokens": if will_fold {
            plan.chunk_tokens() + extraction.report.tokens.tail
        } else {
            0
        },
    });

    if global.json {
        out_json(&report)?;
    } else {
        out(&format!(
            "session:        {}\npath:           {}\nevents:         {} ({} active, {} user turns)\n\
             masked rows:    {} ({} tokens)\nepisodes:       {}\nchunks to fold: {} ({} tokens)\n\
             recency tail:   {} rows ({} tokens)\nbackend:        {}\nplanned calls:  {} fold + {} premap\n",
            summary.reference(),
            summary.path.display(),
            extraction.session.events.len(),
            extraction.session.active.len(),
            extraction.session.user_turns(),
            extraction.rows.len(),
            extraction.report.tokens.masked,
            plan.episodes.len(),
            plan.chunks.len(),
            plan.chunk_tokens(),
            plan.tail.len(),
            extraction.report.tokens.tail,
            backend,
            calls,
            premap,
        ));
    }
    Ok(0)
}

/// Redaction can only be disabled where nothing leaves the machine.
fn parse_redact(value: &str, llm: &Selection, global: &GlobalArgs) -> Result<RedactMode> {
    match value {
        "default" => Ok(RedactMode::Default),
        "strict" => Ok(RedactMode::Strict),
        "off" => {
            if matches!(llm, Selection::None) {
                Ok(RedactMode::Off)
            } else {
                global.note(
                    "--redact off only applies with --llm none; keeping redaction on because a \
                     backend will see this session.",
                );
                Ok(RedactMode::Default)
            }
        }
        other => Err(Error::Usage(format!(
            "unknown --redact `{other}` (expected default, strict, or off)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_cannot_be_disabled_when_a_backend_will_see_the_session() {
        let global = GlobalArgs {
            quiet: true,
            ..GlobalArgs::default()
        };
        assert_eq!(
            parse_redact("off", &Selection::None, &global).expect("parse"),
            RedactMode::Off
        );
        assert_eq!(
            parse_redact("off", &Selection::Cli("claude".into()), &global).expect("parse"),
            RedactMode::Default
        );
        assert_eq!(
            parse_redact(
                "off",
                &Selection::Api {
                    provider: "openai".into(),
                    model: None
                },
                &global
            )
            .expect("parse"),
            RedactMode::Default
        );
    }

    #[test]
    fn redaction_levels_parse() {
        let global = GlobalArgs {
            quiet: true,
            ..GlobalArgs::default()
        };
        assert_eq!(
            parse_redact("strict", &Selection::None, &global).expect("parse"),
            RedactMode::Strict
        );
        assert_eq!(
            parse_redact("nope", &Selection::None, &global)
                .expect_err("reject")
                .exit_code(),
            2
        );
    }
}
