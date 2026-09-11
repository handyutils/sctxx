//! `sctxx handoff` — the main line, for a program.
//!
//! The TUI does this with two keypresses; this is the same thing for a caller
//! that is not a person. A coding agent should be able to run one command and
//! get back something it can use, so:
//!
//! ```text
//! sctxx handoff <ref> --to claude --json
//! ```
//!
//! extracts the session **deterministically** (no model, no tokens), writes the
//! artifact, and prints the exact command that would start the receiving agent —
//! program, argv, and working directory. The caller can then spawn it, or pass
//! `--run` and let sctxx do it.
//!
//! With no `--to`, it lists the agents that are installed and what each one's
//! handoff would look like. That is a question worth being able to ask without
//! having decided yet, and it is what the TUI's picker is showing.

use super::{GlobalArgs, out, out_json};
use crate::adapters::discovery;
use crate::agents::seeding::Launch;
use crate::agents::{self, Agent};
use crate::error::{Error, Result};
use crate::ir::AgentKind;
use crate::pipeline::{self, artifact};
use clap::Args;
use std::path::{Path, PathBuf};

/// `sctxx handoff`
#[derive(Debug, Args)]
pub struct HandoffArgs {
    /// Session reference: `[claude|codex|pi:]<id|prefix|last[:N]>` or a path.
    reference: String,

    /// Which agent should continue the work: claude, codex, or pi.
    #[arg(long, value_name = "AGENT")]
    to: Option<String>,

    /// Where to write the artifact (default: `.sctxx` in the session's project).
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,

    /// Start the agent, instead of printing the command that would.
    #[arg(long)]
    run: bool,

    /// Write the artifact even when one for this session is already there.
    #[arg(long)]
    force: bool,
}

pub fn run(args: &HandoffArgs, global: &GlobalArgs) -> Result<i32> {
    let reference = discovery::parse_reference(&args.reference)?;
    // Any project: a handoff is usually for a session from elsewhere.
    let options = global.resolve_options(true, true);
    let summary = discovery::resolve(&reference, &options)?;

    let machine = agents::Machine::this_one();
    let found = machine.detect();
    let installed: Vec<&Agent> = found.iter().filter(|agent| agent.installed()).collect();

    let Some(wanted) = &args.to else {
        // The question "who could continue this?" is worth answering on its own.
        return list(&installed, &summary, global);
    };
    let agent = pick(&installed, wanted)?;

    let destination = match &args.out {
        Some(path) => resolve_destination(path, &summary)?,
        None => default_destination(&summary),
    };

    let (wrote, reused) = ensure_artifact(&summary, &destination, args.force, global)?;
    let launch = Launch::interactive(
        agent,
        &wrote,
        summary
            .cwd
            .as_deref()
            .map(Path::new)
            .filter(|cwd| cwd.is_dir()),
    )?;

    if args.run {
        global.note(&format!("running: {}", launch.display));
        return launch.run();
    }

    let argv: Vec<String> = launch
        .args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    if global.json {
        out_json(&serde_json::json!({
            "session": summary.reference(),
            "artifact": wrote,
            "directory": destination,
            "reused": reused,
            "agent": launch.agent,
            "route": launch.route.label(),
            "fallback": launch.route.is_fallback(),
            "program": launch.program,
            "argv": argv,
            "cwd": launch.cwd,
            "ran": false,
        }))?;
        return Ok(0);
    }

    out(&format!(
        "session:   {}\nartifact:  {}\nagent:     {}{}\nroute:     {}\ncommand:   {}\ncwd:       {}\n",
        summary.reference(),
        wrote.display(),
        agent.label,
        agent
            .version
            .as_deref()
            .map(|version| format!(" {version}"))
            .unwrap_or_default(),
        launch.route.label(),
        launch.display,
        launch.cwd.display(),
    ));
    global.note("run it with `--run`, or spawn the command above yourself");
    Ok(0)
}

/// Answer "which agent could continue this?" without choosing for the caller.
fn list(
    installed: &[&Agent],
    summary: &discovery::SessionSummary,
    global: &GlobalArgs,
) -> Result<i32> {
    if global.json {
        let agents: Vec<serde_json::Value> = installed
            .iter()
            .map(|agent| {
                serde_json::json!({
                    "agent": agent.id,
                    "label": agent.label,
                    "version": agent.version,
                    "seeding_verified": agent.version_verified(),
                    "verified_against": agent.verified_against,
                    "status": agent.status(),
                })
            })
            .collect();
        out_json(&serde_json::json!({
            "session": summary.reference(),
            "session_path": summary.path,
            "agents": agents,
        }))?;
        return Ok(0);
    }

    if installed.is_empty() {
        global.note("no agent CLI found on PATH (looked for claude, codex, pi)");
        return Ok(0);
    }
    let mut text = format!(
        "session: {}\n\nAgents that could continue it:\n",
        summary.reference()
    );
    for agent in installed {
        text.push_str(&format!("  {:<12} {}\n", agent.id, agent.status()));
    }
    text.push_str("\nchoose one with `--to <agent>`\n");
    out(&text);
    Ok(0)
}

/// The agent named by `--to`, or a refusal that lists what is installed.
fn pick<'a>(installed: &[&'a Agent], wanted: &str) -> Result<&'a Agent> {
    // Accept the reference prefixes too, so `claude-code` works like `claude`.
    let wanted = AgentKind::from_slug(wanted)
        .map(|kind| kind.slug())
        .unwrap_or(wanted);
    if let Some(agent) = installed.iter().find(|agent| agent.id == wanted) {
        return Ok(agent);
    }
    let available: Vec<&str> = installed.iter().map(|agent| agent.id).collect();
    Err(Error::Usage(if available.is_empty() {
        format!(
            "`{wanted}` cannot be launched because no agent CLI was found on PATH \
             (looked for {})",
            agents::known_agents().join(", ")
        )
    } else {
        format!(
            "`{wanted}` is not installed; the agents found here are {}",
            available.join(", ")
        )
    }))
}

/// Where a relative `--out` lands: under the session's project, which is what
/// `.sctxx` means to someone handing work on.
fn resolve_destination(path: &Path, summary: &discovery::SessionSummary) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let base = summary
        .cwd
        .as_deref()
        .map(Path::new)
        .filter(|cwd| cwd.is_dir())
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| Error::Usage("could not resolve a directory for --out".to_string()))?;
    Ok(base.join(path))
}

fn default_destination(summary: &discovery::SessionSummary) -> PathBuf {
    resolve_destination(Path::new(".sctxx"), summary).unwrap_or_else(|_| PathBuf::from(".sctxx"))
}

/// Make sure a handoff for this session is on disk, and say whether it already
/// was.
///
/// Reused rather than rewritten when it belongs to this session: asking for a
/// handoff of something already extracted should not redo the work.
fn ensure_artifact(
    summary: &discovery::SessionSummary,
    destination: &Path,
    force: bool,
    global: &GlobalArgs,
) -> Result<(PathBuf, bool)> {
    let handoff = destination.join("handoff.md");
    if !force
        && artifact::source_reference(&handoff).as_deref() == Some(summary.reference().as_str())
    {
        global.note(&format!("reusing {}", handoff.display()));
        return Ok((handoff, true));
    }

    // Deterministic, and deliberately: this is what the deterministic artifact
    // is for, and a caller asking for context should not be charged for 81 model
    // calls it did not ask for.
    let options: pipeline::ExtractOptions =
        crate::cli::extract::ExtractArgs::deterministic(global, &summary.reference())?;
    let mut progress = |stage: &str, message: &str| {
        global.note(&format!("[{stage}] {message}"));
    };
    let extraction = pipeline::extract(summary, &options, &mut progress)?;
    let written = pipeline::write_destination(&extraction, &options, destination)?;

    // The same warning the CLI has always given, from the same function.
    if written.directory
        && let Some(warning) = crate::cli::extract::git_track_warning(destination)
    {
        global.note(&warning);
    }
    Ok((written.handoff, false))
}
