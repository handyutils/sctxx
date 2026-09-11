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
    #[arg(long, default_value = "standard")]
    mode: String,

    /// Backend: none, auto, cli:<agent>, api:<provider>[/<model>], or mock.
    #[arg(long, default_value = "auto")]
    llm: String,

    /// Artifact token budget, excluding the recency tail.
    #[arg(long, default_value_t = 8_000)]
    budget: usize,

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
    #[arg(long, default_value = "md")]
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
    #[arg(long, default_value = "default")]
    redact: String,

    /// Parallel premap calls.
    #[arg(long, default_value_t = 4)]
    concurrency: usize,

    /// Progress format on stderr: text or json.
    #[arg(long, default_value = "text")]
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
}

pub fn run(args: &ExtractArgs, global: &GlobalArgs) -> Result<i32> {
    let mode = Mode::parse(&args.mode)?;
    let llm = Selection::parse(&args.llm)?;
    let redact = parse_redact(&args.redact, &llm, global)?;
    let layers = render::Layers::parse(&args.layers)?;
    if mode == Mode::Full {
        global
            .note("note: --mode full currently behaves as standard; the probe loop is roadmap M5.");
    }

    let options = ExtractOptions {
        mode,
        llm: llm.clone(),
        budget: args.budget,
        tail_tokens: args.tail,
        chunk_tokens: args.chunk_tokens,
        focus: args.focus.clone(),
        repo: args.repo.clone(),
        verify: !args.no_verify,
        strict: args.strict,
        layers,
        include_sidechains: args.include_sidechains,
        since_compact: args.since_compact,
        keep_reasoning: args.keep_reasoning,
        keep_system: args.keep_system,
        redact,
        concurrency: args.concurrency,
        ..ExtractOptions::default()
    };

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
    let extension = path
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned());
    match extension.as_deref() {
        Some("md") => {
            write_file(path, &extraction.markdown(options))?;
            global.note(&format!("wrote {}", path.display()));
        }
        Some("json") => {
            let json = serde_json::to_string_pretty(&extraction.json(options))
                .map_err(|error| Error::Other(error.to_string()))?;
            write_file(path, &json)?;
            global.note(&format!("wrote {}", path.display()));
        }
        _ => {
            let written = pipeline::write_all(extraction, options, path)?;
            global.note(&format!(
                "wrote {}",
                written
                    .paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<String>>()
                    .join(", ")
            ));
            // The path the receiving agent should read is the payload.
            out(&path.join("handoff.md").to_string_lossy());
            return Ok(0);
        }
    }
    out(&path.to_string_lossy());
    Ok(0)
}

fn write_file(path: &std::path::Path, content: &str) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| Error::io(parent, source))?;
    }
    std::fs::write(path, content).map_err(|source| Error::io(path, source))
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
    let calls = plan.chunks.len() + usize::from(!plan.tail.is_empty());
    let premap = if plan.chunks.len() > 4 {
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
        "estimated_prompt_tokens": plan.chunk_tokens() + extraction.report.tokens.tail,
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
