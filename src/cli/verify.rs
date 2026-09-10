//! `sctxx verify` — re-check an existing artifact against the repository.
//!
//! An artifact ages: files move, commits get rebased, work continues. Verify
//! answers "is this handoff still true?" without re-running extraction.

use super::{GlobalArgs, out, out_json};
use crate::error::{Error, Result};
use crate::pipeline::fold::state::FoldState;
use crate::pipeline::ledgers::Ledgers;
use crate::pipeline::reconcile;
use clap::Args;
use std::path::{Path, PathBuf};

/// `sctxx verify`
#[derive(Debug, Args)]
pub struct VerifyArgs {
    /// Path to `handoff.json`, or the `.sctxx/` directory holding it.
    artifact: PathBuf,

    /// Repository to check against (default: the artifact's recorded cwd).
    #[arg(long, value_name = "PATH")]
    repo: Option<PathBuf>,

    /// Exit 7 when the repository contradicts the artifact.
    #[arg(long)]
    strict: bool,
}

pub fn run(args: &VerifyArgs, global: &GlobalArgs) -> Result<i32> {
    let (handoff_path, dir) = locate(&args.artifact)?;
    let body = std::fs::read_to_string(&handoff_path)
        .map_err(|source| Error::io(&handoff_path, source))?;
    let handoff: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| Error::Other(format!("{}: {error}", handoff_path.display())))?;

    // Ledgers and state are written next to handoff.json; without them there
    // is nothing to reconcile.
    let ledgers: Ledgers = read_json(&dir.join("ledgers.json"))
        .or_else(|| {
            handoff.get("ledgers").and_then(|value| serde_json::from_value(value.clone()).ok())
        })
        .ok_or_else(|| {
            Error::Other(format!(
                "{} has no ledgers; re-run `sctxx extract --out <dir>` to produce a verifiable artifact",
                dir.display()
            ))
        })?;
    let mut state: FoldState = read_json(&dir.join("state.json")).unwrap_or_default();

    let session = rebuild_session(&handoff);
    let repo = args
        .repo
        .clone()
        .or_else(|| session.meta.cwd.clone().filter(|cwd| cwd.is_dir()))
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| Error::Usage("could not determine a repository to verify against".into()))?;

    let reconciliation = reconcile::run(&session, &ledgers, &mut state, &repo);

    if global.json {
        out_json(&reconciliation)?;
    } else {
        out(&summary(&reconciliation));
    }

    // Rewrite the verification block so the artifact carries fresh truth.
    let state_path = dir.join("state.json");
    if state_path.exists()
        && let Ok(text) = serde_json::to_string_pretty(&state)
    {
        let _ = std::fs::write(&state_path, text);
        global.note(&format!("updated {}", state_path.display()));
    }

    if args.strict && reconciliation.contradictions() > 0 {
        return Err(Error::Contradicted(reconciliation.contradictions()));
    }
    Ok(0)
}

fn locate(candidate: &Path) -> Result<(PathBuf, PathBuf)> {
    if candidate.is_dir() {
        let handoff = candidate.join("handoff.json");
        if handoff.is_file() {
            return Ok((handoff, candidate.to_path_buf()));
        }
        return Err(Error::Usage(format!(
            "{} has no handoff.json",
            candidate.display()
        )));
    }
    if !candidate.is_file() {
        return Err(Error::Usage(format!(
            "{} does not exist",
            candidate.display()
        )));
    }
    let dir = candidate.parent().unwrap_or(Path::new(".")).to_path_buf();
    Ok((candidate.to_path_buf(), dir))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let body = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&body).ok()
}

/// Reconstruct just enough of a [`crate::ir::Session`] to reconcile: the
/// metadata and identity. Reconciliation never reads events.
fn rebuild_session(handoff: &serde_json::Value) -> crate::ir::Session {
    let session = &handoff["session"];
    let agent = session["agent"]
        .as_str()
        .and_then(crate::ir::AgentKind::from_slug)
        .unwrap_or(crate::ir::AgentKind::ClaudeCode);
    let meta: crate::ir::SessionMeta = serde_json::from_value(session.clone()).unwrap_or_default();
    crate::ir::Session {
        agent,
        id: session["id"].as_str().unwrap_or_default().to_string(),
        source_paths: session["source_paths"]
            .as_array()
            .map(|paths| {
                paths
                    .iter()
                    .filter_map(|p| p.as_str())
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default(),
        source_hash: session["source_hash"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        meta,
        events: Vec::new(),
        active: Vec::new(),
        native_compactions: Vec::new(),
        diagnostics: Vec::new(),
    }
}

fn summary(report: &reconcile::Reconciliation) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "repo:                 {}\n",
        report
            .repo
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "none".into())
    ));
    if let Some(head) = &report.head {
        out.push_str(&format!("head:                 {head}\n"));
    }
    if let Some(branch) = &report.branch {
        out.push_str(&format!("branch:               {branch}"));
        match &report.session_branch {
            Some(session_branch) if session_branch != branch => {
                out.push_str(&format!("  (session was on {session_branch})\n"));
            }
            _ => out.push('\n'),
        }
    }
    out.push_str(&format!("stale:                {}\n", report.stale()));
    out.push_str(&format!(
        "contradicted:         {}\n",
        report.contradictions()
    ));
    out.push_str(&format!(
        "commits since:        {}\n",
        report.commits_since_session.len()
    ));
    out.push_str(&format!(
        "uncommitted changes:  {}\n",
        report.uncommitted_changes
    ));
    for file in &report.contradicted_files {
        out.push_str(&format!("contradicted: {file} still exists\n"));
    }
    for file in &report.missing_files {
        out.push_str(&format!("stale: {file} is gone\n"));
    }
    for file in &report.changed_since_session {
        out.push_str(&format!("stale: {file} changed after the session\n"));
    }
    for sha in &report.missing_commits {
        out.push_str(&format!(
            "stale: commit {sha} is no longer in this repository\n"
        ));
    }
    if let Some(note) = &report.note {
        out.push_str(&format!("note: {note}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_without_an_artifact_is_a_usage_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(locate(dir.path()).expect_err("reject").exit_code(), 2);
    }

    #[test]
    fn a_directory_with_an_artifact_locates_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("handoff.json"), "{}").expect("write");
        let (handoff, parent) = locate(dir.path()).expect("locate");
        assert!(handoff.ends_with("handoff.json"));
        assert_eq!(parent, dir.path());
    }

    #[test]
    fn the_session_identity_is_recovered_from_the_artifact() {
        let handoff = serde_json::json!({
            "session": {
                "agent": "codex",
                "id": "abc",
                "source_hash": "deadbeef",
                "cwd": "/repo",
                "git_branch": "main"
            }
        });
        let session = rebuild_session(&handoff);
        assert_eq!(session.agent, crate::ir::AgentKind::Codex);
        assert_eq!(session.id, "abc");
        assert_eq!(session.meta.git_branch.as_deref(), Some("main"));
    }
}
