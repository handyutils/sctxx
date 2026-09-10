//! `sctxx skill` — install, remove, or print the Agent Skill (spec §13).

use super::{GlobalArgs, out, out_json};
use crate::error::{Error, Result};
use crate::skill::{self, Outcome, Scope, Target};
use clap::{Args, Subcommand};
use std::path::PathBuf;

/// `sctxx skill <command>`
#[derive(Debug, Subcommand)]
pub enum SkillCommand {
    /// Write SKILL.md into the agents' skill directories.
    Install(InstallArgs),
    /// Remove an installed skill.
    Uninstall(InstallArgs),
    /// Print SKILL.md on stdout.
    Print,
}

/// Shared arguments for install and uninstall.
#[derive(Debug, Args)]
pub struct InstallArgs {
    /// Agents to install for; repeatable. Default: all detected.
    #[arg(long = "target", value_name = "AGENT")]
    targets: Vec<String>,

    /// user (default) or project.
    #[arg(long, default_value = "user")]
    scope: String,

    /// Overwrite a SKILL.md that was modified locally.
    #[arg(long)]
    force: bool,

    /// Project directory for `--scope project` (default: the current one).
    #[arg(long, value_name = "PATH")]
    project: Option<PathBuf>,
}

pub fn run(command: &SkillCommand, global: &GlobalArgs) -> Result<i32> {
    match command {
        SkillCommand::Print => {
            out(skill::SKILL_MD);
            Ok(0)
        }
        SkillCommand::Install(args) => apply(args, global, true),
        SkillCommand::Uninstall(args) => apply(args, global, false),
    }
}

fn apply(args: &InstallArgs, global: &GlobalArgs, installing: bool) -> Result<i32> {
    let scope = Scope::parse(&args.scope)?;
    let targets = if args.targets.is_empty() {
        Target::ALL.to_vec()
    } else {
        args.targets
            .iter()
            .map(|name| Target::parse(name))
            .collect::<Result<Vec<Target>>>()?
    };
    let home = home_dir()?;
    let project = args
        .project
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    let results = if installing {
        skill::install(&targets, scope, &home, &project, args.force)
    } else {
        skill::uninstall(&targets, scope, &home, &project)
    };

    if global.json {
        let payload: Vec<serde_json::Value> = results
            .iter()
            .map(|(target, outcome)| {
                serde_json::json!({ "target": target.slug(), "outcome": outcome })
            })
            .collect();
        out_json(&payload)?;
        return Ok(exit_code(&results));
    }

    let mut text = String::new();
    for (target, outcome) in &results {
        let line = match outcome {
            Outcome::Installed { path } => format!("installed  {} → {path}", target.label()),
            Outcome::Updated { path, from } => {
                format!("updated    {} → {path} (was {from})", target.label())
            }
            Outcome::Unchanged { path } => format!("unchanged  {} → {path}", target.label()),
            Outcome::Modified { path } => format!(
                "skipped    {} → {path} was modified locally; pass --force to overwrite",
                target.label()
            ),
            Outcome::Removed { path } => format!("removed    {} → {path}", target.label()),
            Outcome::Skipped { reason } => format!("skipped    {}: {reason}", target.label()),
        };
        text.push_str(&line);
        text.push('\n');
    }
    out(&text);
    if installing
        && results
            .iter()
            .any(|(_, outcome)| matches!(outcome, Outcome::Installed { .. }))
    {
        global.note(
            "the skill triggers on phrases like \"continue the session from Claude Code\"; \
             restart the agent if it does not pick it up.",
        );
    }
    Ok(exit_code(&results))
}

/// A locally modified skill is the one outcome worth a nonzero exit, so a
/// scripted install does not silently do nothing.
fn exit_code(results: &[(Target, Outcome)]) -> i32 {
    if results
        .iter()
        .any(|(_, outcome)| matches!(outcome, Outcome::Modified { .. }))
    {
        return 1;
    }
    0
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| Error::Other("could not determine the home directory".into()))
}
