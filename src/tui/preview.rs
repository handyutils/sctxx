//! What one session contains, read from the deterministic ledgers.
//!
//! The preview is how a developer decides whether a session is worth extracting
//! at all, so it answers the question that matters without a model: how many
//! human turns, which files the work was about, what the last command did, what
//! is still broken, and how much of the transcript survives its own rewinds and
//! the provider's compactions (spec §7.2, FR-009).
//!
//! Reading a session is seconds of work, not a keystroke, so nothing here runs
//! on the thread that draws. [`super::work`] owns the thread; this module owns
//! the work, and stays synchronous, which is what makes it testable.

use crate::adapters::{self, discovery::SessionSummary, source};
use crate::error::{Error, Result};
use crate::ir::{AgentKind, Session};
use crate::pipeline::ledgers::{self, FileRecord, Ledgers};
use crate::vendor::codex::secrets::RedactMode;

/// How many paths and error signatures the pane lists before it summarises.
const SHOWN: usize = 8;

/// An owned, small view of one session's ledgers.
///
/// Owned and small on purpose: it crosses a thread boundary and is cached, so
/// holding the session or the ledger vectors alive would pin a transcript in
/// memory for every session the developer browsed past.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LedgerPreview {
    /// The first human message: what the session was asked to do.
    pub goal: Option<String>,
    /// Human turns on the active branch, so a rewound conversation is not
    /// counted as if every attempt were still live.
    pub user_turns: usize,
    pub files_touched: usize,
    /// The most-edited and most-read paths, busiest first.
    pub top_files: Vec<String>,
    pub last_command: Option<String>,
    pub last_command_status: Option<&'static str>,
    pub unresolved_errors: usize,
    /// Unresolved error signatures, most frequent first.
    pub errors: Vec<String>,
    pub compactions: usize,
    /// Compactions that *discarded* history rather than re-anchoring the
    /// window, because that is the difference between "the provider tidied up"
    /// and "the early history is gone" (ADR 0002).
    pub compaction_resets: usize,
    pub live_events: usize,
    pub total_events: usize,
    pub diagnostics: usize,
    /// The first diagnostic, which is the one worth reading.
    pub diagnostic: Option<String>,
}

impl LedgerPreview {
    /// True when the session read cleanly.
    pub fn is_clean(&self) -> bool {
        self.diagnostics == 0
    }
}

/// Read one session's ledgers.
///
/// Synchronous and deterministic. This is the work the worker thread exists to
/// carry, and the only entry point to it: the pane never calls it directly.
pub fn load(summary: &SessionSummary) -> Result<LedgerPreview> {
    let agent = AgentKind::from_slug(summary.agent).ok_or_else(|| {
        Error::Usage(format!(
            "`{}` is not an agent sctxx can read",
            summary.agent
        ))
    })?;
    let text = source::read(&summary.path)?;
    // The same tolerance the CLI uses, so a preview and an extract agree about
    // whether a session is readable at all.
    let session = adapters::parse_as(agent, text, adapters::DEFAULT_MAX_BAD_LINE_RATE)?;
    // Masked like anything else sctxx renders. A preview quotes real command
    // lines and paths, and the masks cost nothing to keep consistent with the
    // artifact that would follow.
    let ledgers = ledgers::build(&session, RedactMode::Default);
    Ok(summarize(&session, &ledgers))
}

/// Reduce a parsed session and its ledgers to what the pane shows.
fn summarize(session: &Session, ledgers: &Ledgers) -> LedgerPreview {
    let mut by_activity: Vec<&FileRecord> = ledgers.files.iter().collect();
    // Busiest first, ties broken by path: the order must not depend on how the
    // ledger happened to be built.
    by_activity.sort_by(|a, b| {
        (b.edits + b.reads)
            .cmp(&(a.edits + a.reads))
            .then_with(|| a.path.cmp(&b.path))
    });

    let last_command = ledgers.last_command_status().into_iter().next();
    let unresolved = ledgers.unresolved_errors();

    LedgerPreview {
        goal: ledgers.user_messages.first().map(|m| m.text.clone()),
        user_turns: session.user_turns(),
        files_touched: by_activity.len(),
        top_files: by_activity
            .iter()
            .take(SHOWN)
            .map(|file| file.path.clone())
            .collect(),
        last_command: last_command.map(|record| record.command.clone()),
        last_command_status: last_command.map(|record| record.status()),
        unresolved_errors: unresolved.len(),
        errors: unresolved
            .iter()
            .take(SHOWN)
            .map(|error| error.example.clone())
            .collect(),
        compactions: session.native_compactions.len(),
        compaction_resets: session
            .native_compactions
            .iter()
            .filter(|compaction| !compaction.windowed)
            .count(),
        live_events: session.active.len(),
        total_events: session.events.len(),
        diagnostics: session.diagnostics.len(),
        diagnostic: session.diagnostics.first().map(|d| d.label()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real, redacted Claude Code fixture: the preview must work on the same
    /// shape the adapter is tested against, not on a hand-built `Session`.
    fn fixture(name: &str) -> SessionSummary {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/claude")
            .join(name);
        SessionSummary {
            agent: "claude",
            id: "fixture".into(),
            path,
            cwd: None,
            started_at: None,
            ended_at: None,
            lines: 0,
            bytes: 0,
            title: None,
            first_message: None,
            mtime: 0,
        }
    }

    #[test]
    fn a_real_fixture_yields_ledgers_without_a_model() {
        let preview = load(&fixture("basic.jsonl")).expect("fixture should read");
        assert!(
            preview.user_turns > 0,
            "a session with human turns must report them"
        );
        assert!(preview.total_events > 0);
        assert!(
            preview.live_events <= preview.total_events,
            "the active branch can never exceed the whole transcript"
        );
        assert!(
            preview.goal.is_some(),
            "the first human message is the goal"
        );
    }

    #[test]
    fn the_file_and_error_lists_are_bounded_but_counted_in_full() {
        let preview = load(&fixture("basic.jsonl")).expect("fixture should read");
        assert!(preview.top_files.len() <= SHOWN);
        assert!(preview.errors.len() <= SHOWN);
        // The counts are what the pane states; the lists are what it shows.
        assert!(preview.files_touched >= preview.top_files.len());
        assert!(preview.unresolved_errors >= preview.errors.len());
    }

    #[test]
    fn a_missing_file_is_an_error_rather_than_an_empty_preview() {
        let mut summary = fixture("basic.jsonl");
        summary.path = std::path::Path::new("/nonexistent/nope.jsonl").to_path_buf();
        assert!(
            load(&summary).is_err(),
            "a vanished session must not read as empty"
        );
    }

    #[test]
    fn an_agent_sctxx_cannot_read_is_rejected_before_touching_the_disk() {
        let mut summary = fixture("basic.jsonl");
        summary.agent = "notanagent";
        let error = load(&summary).expect_err("unknown agent");
        assert_eq!(error.exit_code(), 2);
    }
}
