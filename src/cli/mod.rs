//! The command-line surface (spec §3).
//!
//! Two conventions matter here and are enforced by tests:
//!
//! * **stdout carries the payload only** — the artifact, the JSON, the event
//!   text. Progress, warnings, and diagnostics go to stderr, so an agent can
//!   pipe stdout straight into a file or a parser.
//! * **exit codes are a contract** (`docs/SCTXX-SPEC.md` §3.1). Agents branch
//!   on them: 3 means "ambiguous, here are the candidates", 4 means "not
//!   found", 6 means "no LLM backend".

mod discover;
mod doctor;
mod extract;
mod redact;
mod schema;
mod skill;
mod verify;

use crate::adapters::discovery::{ResolveOptions, Roots};
use crate::error::{Error, Result};
use clap::{Args, Parser, Subcommand};
use std::io::Write;
use std::path::PathBuf;

/// Turn a coding-agent session into a handoff another agent can continue from.
#[derive(Debug, Parser)]
#[command(
    name = "sctxx",
    version,
    about = "Session ConTeXt eXtractor: turn a coding-agent session into a verified handoff artifact.",
    long_about = None,
    after_help = "Docs: https://handyutils.github.io/sctxx\nRun `sctxx doctor` to see which session stores and LLM backends were detected."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,

    #[command(flatten)]
    global: GlobalArgs,
}

/// Flags every subcommand accepts.
#[derive(Debug, Args, Clone, Default)]
pub struct GlobalArgs {
    /// Machine-readable output on stdout.
    #[arg(long, global = true)]
    pub json: bool,

    /// Suppress progress and diagnostics on stderr.
    #[arg(long, global = true)]
    pub quiet: bool,

    /// Claude Code projects directory (default: ~/.claude/projects).
    #[arg(long, global = true, value_name = "DIR")]
    pub claude_root: Option<PathBuf>,

    /// Codex home or sessions directory (default: ~/.codex).
    #[arg(long, global = true, value_name = "DIR")]
    pub codex_root: Option<PathBuf>,

    /// Pi sessions directory (default: ~/.pi/agent/sessions).
    #[arg(long, global = true, value_name = "DIR")]
    pub pi_root: Option<PathBuf>,
}

impl GlobalArgs {
    pub fn roots(&self) -> Roots {
        Roots {
            claude: self.claude_root.clone(),
            codex: self.codex_root.clone(),
            pi: self.pi_root.clone(),
        }
    }

    pub fn resolve_options(&self, any_project: bool, up: bool) -> ResolveOptions {
        ResolveOptions {
            roots: self.roots(),
            any_project,
            up,
            cwd: None,
        }
    }

    /// Write a diagnostic to stderr unless `--quiet`.
    pub fn note(&self, message: &str) {
        if !self.quiet {
            let _ = writeln!(std::io::stderr(), "{message}");
        }
    }
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List sessions found in the agents' stores, newest first.
    List(discover::ListArgs),
    /// Search sessions by title, first message, or user messages.
    Find(discover::FindArgs),
    /// Print a session's events as raw JSON, masked rows, or canonical IR.
    Show(discover::ShowArgs),
    /// Produce a handoff artifact from a session. The main command.
    Extract(extract::ExtractArgs),
    /// Print the events behind an `[evt a-b]` pointer from an artifact.
    Expand(discover::ExpandArgs),
    /// Re-check an existing artifact against the current repository.
    Verify(verify::VerifyArgs),
    /// Redact secrets from a session file, for contributing a fixture.
    Redact(redact::RedactArgs),
    /// Install, remove, or print the Agent Skill.
    #[command(subcommand)]
    Skill(skill::SkillCommand),
    /// Print a JSON Schema for one of sctxx's contracts.
    Schema(schema::SchemaArgs),
    /// Report detected session stores, LLM backends, and configuration.
    Doctor,
}

/// Parse arguments and run. Returns the process exit code.
pub fn run() -> i32 {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            // clap writes help and version to stdout, errors to stderr, and
            // knows which is which.
            let _ = error.print();
            return match error.kind() {
                clap::error::ErrorKind::DisplayHelp
                | clap::error::ErrorKind::DisplayVersion
                | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => 0,
                _ => 2,
            };
        }
    };

    match dispatch(&cli) {
        Ok(code) => code,
        Err(error) => report(&error, cli.global.json),
    }
}

fn dispatch(cli: &Cli) -> Result<i32> {
    let global = &cli.global;
    match &cli.command {
        Command::List(args) => discover::list(args, global),
        Command::Find(args) => discover::find(args, global),
        Command::Show(args) => discover::show(args, global),
        Command::Extract(args) => extract::run(args, global),
        Command::Expand(args) => discover::expand(args, global),
        Command::Verify(args) => verify::run(args, global),
        Command::Redact(args) => redact::run(args, global),
        Command::Skill(command) => skill::run(command, global),
        Command::Schema(args) => schema::run(args, global),
        Command::Doctor => doctor::run(global),
    }
}

/// Print an error the way its exit code implies, and return that code.
fn report(error: &Error, json: bool) -> i32 {
    // Ambiguity is not a failure to explain in prose: the candidate list is
    // the payload an agent needs, so it goes to stdout as JSON.
    if let Error::Ambiguous {
        reference,
        count,
        candidates_json,
    } = error
    {
        let _ = writeln!(std::io::stdout(), "{candidates_json}");
        let _ = writeln!(
            std::io::stderr(),
            "error: `{reference}` matched {count} sessions; the candidates are on stdout"
        );
        return error.exit_code();
    }
    if json {
        let payload = serde_json::json!({
            "error": error.to_string(),
            "exit_code": error.exit_code(),
        });
        let _ = writeln!(std::io::stderr(), "{payload}");
    } else {
        let _ = writeln!(std::io::stderr(), "error: {error}");
    }
    error.exit_code()
}

/// Write a payload to stdout.
pub(crate) fn out(text: &str) {
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(text.as_bytes());
    if !text.ends_with('\n') {
        let _ = stdout.write_all(b"\n");
    }
    let _ = stdout.flush();
}

/// Write JSON to stdout.
pub(crate) fn out_json<T: serde::Serialize>(value: &T) -> Result<()> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| Error::Other(format!("could not serialize output: {error}")))?;
    out(&text);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_command_tree_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn every_documented_subcommand_exists() {
        let command = Cli::command();
        let names: Vec<&str> = command
            .get_subcommands()
            .map(|sub| sub.get_name())
            .collect();
        for expected in [
            "list", "find", "show", "extract", "expand", "verify", "redact", "skill", "schema",
            "doctor",
        ] {
            assert!(
                names.contains(&expected),
                "missing subcommand {expected} in {names:?}"
            );
        }
    }

    #[test]
    fn global_flags_are_accepted_after_a_subcommand() {
        let cli = Cli::try_parse_from(["sctxx", "list", "--json", "--quiet"]).expect("parse");
        assert!(cli.global.json && cli.global.quiet);
    }

    #[test]
    fn ambiguity_puts_candidates_on_stdout_and_exits_three() {
        let error = Error::Ambiguous {
            reference: "abc123".into(),
            count: 2,
            candidates_json: "[]".into(),
        };
        assert_eq!(report(&error, false), 3);
    }
}
