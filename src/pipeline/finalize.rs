//! End-state reconciliation: what the evidence says after the fold has spoken.
//!
//! The fold reads chunks in order and the last thing it sees is the recency tail,
//! which makes it good at *what happened* and bad at *what is true now*. An action
//! that a later command satisfied is still sitting in the state as pending, and a
//! current step captured mid-session is presented as current. That is the shape of
//! the largest error a reviewer found in a real artifact.
//!
//! This pass runs after the fold and before rendering, over evidence the fold
//! cannot argue with — the ledger's commands and the repository — and it is
//! entirely deterministic. **No model is called here, and none is needed.**
//!
//! Two rules govern everything below:
//!
//! 1. **Nothing is resolved silently.** Every resolution cites the command and the
//!    event that resolved it, and the artifact lists them, so a reader can check
//!    the decision instead of trusting it.
//! 2. **A finding is a claim about the session, not about the repository.** The
//!    session said X; the repository now says Y; both are reported.

use super::fold::ops::ItemKind;
use super::fold::state::{FoldState, ItemStatus};
use super::ledgers::Ledgers;
use super::reconcile::Reconciliation;
use crate::ir::EventIdx;

/// The shortest command that may be matched inside an item's text.
///
/// Below this, a "match" is a coincidence: `ls`, `cd`, `go` appear in prose.
const MIN_MATCHED_COMMAND: usize = 5;

/// How a stale-claim count stops being metadata and becomes a warning.
const STALE_FRACTION_WARNING: f64 = 0.25;

/// An action the evidence resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub id: String,
    /// The action as the fold wrote it, quoted in the artifact.
    pub text: String,
    /// The command that satisfied it.
    pub command: String,
    /// The event that satisfied it.
    pub evt: EventIdx,
}

/// A way the evidence contradicts the artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingKind {
    /// The repository moved on after the session ended.
    RepoMovedOn,
    /// Files the artifact cites are gone.
    MissingFiles,
    /// Files the session said it deleted are still there.
    ContradictedFiles,
    /// The working tree is dirty now, which the artifact cannot know.
    UncommittedChanges,
    /// Claims that verified against the repository during the session are stale.
    StaleClaims,
    /// Commits the session recorded are no longer reachable from HEAD.
    MissingCommits,
}

impl FindingKind {
    pub fn label(self) -> &'static str {
        match self {
            FindingKind::RepoMovedOn => "the repository moved on",
            FindingKind::MissingFiles => "cited files are gone",
            FindingKind::ContradictedFiles => "the session was contradicted",
            FindingKind::UncommittedChanges => "the working tree has changed",
            FindingKind::StaleClaims => "verified claims went stale",
            FindingKind::MissingCommits => "the history was rewritten",
        }
    }
}

/// One contradiction, ready to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub kind: FindingKind,
    /// One sentence a reader can act on.
    pub text: String,
}

/// A current step derived from the evidence, when the fold supplied none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedStep {
    pub text: String,
    pub evt: EventIdx,
}

/// What the end-state pass concluded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EndState {
    pub resolutions: Vec<Resolution>,
    pub findings: Vec<Finding>,
    pub derived_step: Option<DerivedStep>,
}

impl EndState {
    pub fn is_empty(&self) -> bool {
        self.resolutions.is_empty() && self.findings.is_empty() && self.derived_step.is_none()
    }
}

/// Reconcile the folded state against the evidence, mutating it and reporting.
///
/// Mutation is limited to marking actions resolved: an item that the ledger shows
/// was satisfied is not pending any more, and leaving it pending is what makes a
/// receiving agent redo work.
pub fn run(state: &mut FoldState, ledgers: &Ledgers, reconciliation: &Reconciliation) -> EndState {
    let mut out = EndState {
        resolutions: resolve_satisfied_actions(state, ledgers),
        findings: findings(state, ledgers, reconciliation),
        derived_step: derived_step(state, ledgers),
    };
    out.findings.sort_by_key(|finding| finding.kind.label());
    out
}

/// Mark every action a later, successful command satisfied.
///
/// The match is deliberately literal: the item's text has to contain the
/// command, and the command has to have run *after* the item's evidence and
/// succeeded. A near-miss is left alone, because a wrong resolution is worse
/// than a pending action.
fn resolve_satisfied_actions(state: &mut FoldState, ledgers: &Ledgers) -> Vec<Resolution> {
    let mut resolutions = Vec::new();
    // Collected first, because applying needs `&mut state` and scanning needs it
    // borrowed.
    let candidates: Vec<(String, String, EventIdx)> = state
        .items
        .iter()
        .filter(|item| item.kind == ItemKind::NextAction && item.status.is_active())
        .filter_map(|item| {
            let after = item
                .sources
                .iter()
                .map(|range| range.end)
                .max()
                .unwrap_or(item.last_confirmed);
            let command = ledgers
                .commands
                .iter()
                .filter(|command| command.evt > after && !command.failed())
                .find(|command| names_command(&item.text, &command.normalized))?;
            Some((item.id.clone(), item.text.clone(), command.evt))
        })
        .collect();

    for (id, text, evt) in candidates {
        if let Some(item) = state.items.iter_mut().find(|item| item.id == id) {
            item.status = ItemStatus::Resolved { evt };
            resolutions.push(Resolution {
                id,
                text,
                command: ledgers
                    .commands
                    .iter()
                    .find(|command| command.evt == evt)
                    .map(|command| command.normalized.clone())
                    .unwrap_or_default(),
                evt,
            });
        }
    }
    resolutions.sort_by_key(|resolution| (resolution.evt, resolution.id.clone()));
    resolutions
}

/// Whether an action's text names this command.
///
/// Literal containment, case-insensitive, on a command long enough that the match
/// means something. Both sides are whitespace-collapsed first, because the fold
/// writes prose and the ledger writes a command line.
fn names_command(text: &str, command: &str) -> bool {
    let text = collapse(text);
    let command = collapse(command);
    if command.chars().count() < MIN_MATCHED_COMMAND {
        return false;
    }
    text.contains(&command)
}

fn collapse(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
        .to_lowercase()
}

/// Whether a recorded command is a state a reader could act from.
///
/// A heredoc writes a file and exits; it says nothing about where the work is.
fn is_command_like(command: &str) -> bool {
    !command.contains('\n') && !command.contains("<<") && command.chars().count() <= 200
}

/// The first line of a command, for anywhere a command is quoted in prose.
fn first_line(command: &str) -> String {
    command.lines().next().unwrap_or_default().to_string()
}

/// Where the session was when it stopped, from the ledger rather than from a
/// mid-session snapshot.
///
/// Only used when the fold left no current step, and it says that it is derived.
fn derived_step(state: &FoldState, ledgers: &Ledgers) -> Option<DerivedStep> {
    if !state.active_of(ItemKind::CurrentStep).is_empty() {
        return None;
    }
    // The agent's own plan is the best statement of where the work is: it wrote
    // it down deliberately, where the command history only shows what it last
    // typed. Falls back to a command when there is no plan.
    if let Some(plan) = &ledgers.plan
        && let Some(item) = plan
            .items
            .iter()
            .find(|item| matches!(item.status.as_str(), "in_progress" | "active"))
    {
        return Some(DerivedStep {
            text: format!(
                "derived (no model ran), from the agent's own plan: {}",
                item.text
            ),
            evt: plan.evt,
        });
    }

    // Otherwise the *last* command, preferring one a reader could re-run: on a
    // session whose final act was writing a changelog, a heredoc is not a state.
    let last = ledgers
        .commands
        .iter()
        .filter(|command| is_command_like(&command.normalized))
        .max_by_key(|command| command.evt)
        .or_else(|| ledgers.commands.iter().max_by_key(|command| command.evt))?;
    Some(DerivedStep {
        text: format!(
            "derived (no model ran): `{}` was the last real command \u{2014} {}",
            crate::vendor::codex::truncate::truncate_middle_bytes(
                first_line(&last.normalized).as_str(),
                80
            ),
            last.status()
        ),
        evt: last.evt,
    })
}

/// What the evidence says that the artifact does not.
fn findings(state: &FoldState, ledgers: &Ledgers, reconciliation: &Reconciliation) -> Vec<Finding> {
    let mut findings = Vec::new();

    if !reconciliation.commits_since_session.is_empty() {
        let first = reconciliation
            .commits_since_session
            .first()
            .map(String::as_str)
            .unwrap_or_default();
        findings.push(Finding {
            kind: FindingKind::RepoMovedOn,
            text: format!(
                "{} commit(s) landed after this session ended, so the artifact is behind the \
                 repository. Newest: `{}`. Read the working tree before trusting the goal or the \
                 next action.",
                reconciliation.commits_since_session.len(),
                crate::vendor::codex::truncate::truncate_middle_bytes(first, 80)
            ),
        });
    }

    if !reconciliation.contradicted_files.is_empty() {
        findings.push(Finding {
            kind: FindingKind::ContradictedFiles,
            text: format!(
                "the session reported deleting {} file(s) that are still on disk ({}). The session's \
                 record of that work is wrong.",
                reconciliation.contradicted_files.len(),
                sample_owned(&reconciliation.contradicted_files.iter().collect::<Vec<_>>())
            ),
        });
    }

    if !reconciliation.missing_files.is_empty() {
        // Some cited paths are not the work: npm debug logs, temp files, and
        // `node_modules` are things the session *read*, not things it changed.
        // They are counted and named separately, because a finding that leads
        // with a log file teaches a reader to skim findings.
        let (work, noise): (Vec<&String>, Vec<&String>) = reconciliation
            .missing_files
            .iter()
            .partition(|path| looks_like_work(path));
        if !work.is_empty() {
            findings.push(Finding {
                kind: FindingKind::MissingFiles,
                text: format!(
                    "{} file(s) the session worked on no longer exist ({}). Pointers into them will \
                     not resolve.",
                    work.len(),
                    sample_owned(&work)
                ),
            });
        }
        if !noise.is_empty() {
            findings.push(Finding {
                kind: FindingKind::MissingFiles,
                text: format!(
                    "{} cited path(s) are gone that were never the work — logs, temp files, or \
                     dependencies the session read.",
                    noise.len()
                ),
            });
        }
    }

    // The header's `stale` count is the sum of three different things, and on the
    // session this was written from it read as "100 stale claims" when it was 86
    // missing files and 14 unreachable commits. Both are findings now, described
    // for what they are, so the number in the header is never the only word on it.
    if !reconciliation.missing_commits.is_empty() {
        findings.push(Finding {
            kind: FindingKind::MissingCommits,
            text: format!(
                "{} commit(s) this session recorded are no longer reachable from HEAD ({}). The \
                 history was rewritten — rebased, amended, or the branch moved — so line numbers \
                 and diffs quoted in this artifact may be wrong.",
                reconciliation.missing_commits.len(),
                sample_owned(&reconciliation.missing_commits.iter().collect::<Vec<_>>())
            ),
        });
    }

    if reconciliation.uncommitted_changes > 0 {
        findings.push(Finding {
            kind: FindingKind::UncommittedChanges,
            text: format!(
                "{} uncommitted change(s) in the working tree right now. Whether they are this \
                 session's work or someone else's is not something the artifact can know.",
                reconciliation.uncommitted_changes
            ),
        });
    }

    // A large stale count is a warning, not metadata: the review's point was that
    // `stale: 100` in a header is a number nobody acts on.
    let stale = state
        .items
        .iter()
        .filter(|item| item.verified == super::fold::state::Verification::Stale)
        .count();
    let checked = state
        .items
        .iter()
        .filter(|item| item.verified != super::fold::state::Verification::Unchecked)
        .count();
    if stale > 0 {
        let fraction = if checked == 0 {
            0.0
        } else {
            stale as f64 / checked as f64
        };
        let severity = if fraction >= STALE_FRACTION_WARNING {
            "Most of what was checked"
        } else {
            "Some of what was checked"
        };
        findings.push(Finding {
            kind: FindingKind::StaleClaims,
            text: format!(
                "{severity} no longer matches the repository: {stale} of {checked} verified \
                 claim(s) are stale. Treat those items as history, not as current fact."
            ),
        });
    }

    // An action whose command has failed more recently than the action itself is
    // still open, and saying so is cheaper than a receiving agent rediscovering it.
    let repeated = ledgers
        .unresolved_errors()
        .first()
        .map(|error| error.example.clone());
    if let Some(example) = repeated
        && !state.active_of(ItemKind::NextAction).is_empty()
    {
        findings.push(Finding {
            kind: FindingKind::ContradictedFiles,
            text: format!(
                "the session ended with an unresolved failure that an action may still be about: {}",
                crate::vendor::codex::truncate::truncate_middle_bytes(&example, 120)
            ),
        });
    }

    findings
}

/// Whether a cited path is plausibly something the session *worked on*.
///
/// Deliberately a small, readable rule rather than a classifier: paths under a
/// log directory, ending in `.log`, or inside `node_modules` are things a session
/// reads while working, not things it changed.
fn looks_like_work(path: &str) -> bool {
    let lowered = path.to_lowercase();
    const NOISE: &[&str] = &[
        "/logs/",
        "_logs",
        ".log",
        "/node_modules/",
        "/.npm/",
        "/tmp/",
    ];
    !NOISE.iter().any(|marker| lowered.contains(marker))
}

fn sample_owned(paths: &[&String]) -> String {
    let shown: Vec<String> = paths
        .iter()
        .take(3)
        .map(|path| crate::vendor::codex::truncate::truncate_middle_bytes(path, 60).to_string())
        .collect();
    if paths.len() > shown.len() {
        format!(
            "{}, and {} more",
            shown.join(", "),
            paths.len() - shown.len()
        )
    } else {
        shown.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::fold::ops::{Confidence, EvtRange, NewItem, Op};
    use crate::pipeline::fold::state::Verification;
    use crate::pipeline::ledgers::{CmdCategory, CommandRecord};

    fn action(text: &str, evt: EventIdx) -> Op {
        Op::Add {
            item: NewItem {
                kind: ItemKind::NextAction,
                text: text.to_string(),
                why: None,
                quote: None,
                sources: vec![EvtRange::new(evt, evt)],
                rejected: Vec::new(),
                confidence: Confidence::High,
            },
        }
    }

    fn command(evt: EventIdx, normalized: &str, exit: i32) -> CommandRecord {
        CommandRecord {
            evt,
            command: normalized.to_string(),
            normalized: normalized.to_string(),
            exit_code: Some(exit),
            is_error: Some(exit != 0),
            category: CmdCategory::Test,
            output_head: String::new(),
            output_tail: String::new(),
        }
    }

    fn state_with(ops: Vec<Op>) -> FoldState {
        let mut state = FoldState::new();
        for op in ops {
            state.apply("c0", &op);
        }
        state
    }

    #[test]
    fn an_action_a_later_successful_command_satisfied_is_resolved() {
        // The failure this exists for: the receiving agent redoing work the
        // session already finished.
        let mut state = state_with(vec![action("run `cargo test --lib` and fix it", 10)]);
        let ledgers = Ledgers {
            commands: vec![command(40, "cargo test --lib", 0)],
            ..Default::default()
        };
        let end = run(&mut state, &ledgers, &Reconciliation::default());

        assert_eq!(end.resolutions.len(), 1, "{end:?}");
        let resolution = &end.resolutions[0];
        assert_eq!(resolution.command, "cargo test --lib");
        assert_eq!(resolution.evt, 40);
        // And the state really changed: nothing is resolved only on paper.
        assert!(state.active_of(ItemKind::NextAction).is_empty());
    }

    #[test]
    fn an_action_whose_command_failed_is_left_alone() {
        let mut state = state_with(vec![action("run `cargo test --lib`", 10)]);
        let ledgers = Ledgers {
            commands: vec![command(40, "cargo test --lib", 101)],
            ..Default::default()
        };
        let end = run(&mut state, &ledgers, &Reconciliation::default());
        assert!(end.resolutions.is_empty(), "{end:?}");
        assert_eq!(state.active_of(ItemKind::NextAction).len(), 1);
    }

    #[test]
    fn success_before_the_action_does_not_resolve_it() {
        // A command that ran before the action was proposed cannot satisfy it.
        let mut state = state_with(vec![action("run `cargo test --lib`", 50)]);
        let ledgers = Ledgers {
            commands: vec![command(40, "cargo test --lib", 0)],
            ..Default::default()
        };
        let end = run(&mut state, &ledgers, &Reconciliation::default());
        assert!(end.resolutions.is_empty(), "{end:?}");
    }

    #[test]
    fn a_short_command_is_not_matched_inside_prose() {
        // `ls` and `cd` appear in sentences, and a wrong resolution is worse than
        // a pending action.
        let mut state = state_with(vec![action("list the files with ls", 10)]);
        let ledgers = Ledgers {
            commands: vec![command(40, "ls", 0)],
            ..Default::default()
        };
        let end = run(&mut state, &ledgers, &Reconciliation::default());
        assert!(
            end.resolutions.is_empty(),
            "a two-letter command is a coincidence"
        );
    }

    #[test]
    fn a_derived_step_appears_only_when_the_fold_left_none() {
        let ledgers = Ledgers {
            commands: vec![command(90, "cargo build", 1)],
            ..Default::default()
        };

        let mut state = state_with(Vec::new());
        let end = run(&mut state, &ledgers, &Reconciliation::default());
        let derived = end.derived_step.expect("a step derived from the ledger");
        assert!(derived.text.contains("cargo build"), "{}", derived.text);
        assert!(derived.text.contains("FAILED"), "{}", derived.text);
        assert!(derived.text.contains("derived"), "{}", derived.text);
        assert_eq!(derived.evt, 90);

        // With a folded current step, the ledger does not override it.
        let mut state = state_with(vec![Op::Add {
            item: NewItem {
                kind: ItemKind::CurrentStep,
                text: "wire the parser".into(),
                why: None,
                quote: None,
                sources: vec![EvtRange::new(20, 20)],
                rejected: Vec::new(),
                confidence: Confidence::High,
            },
        }]);
        let end = run(&mut state, &ledgers, &Reconciliation::default());
        assert!(end.derived_step.is_none());
    }

    #[test]
    fn the_repository_moving_on_is_reported_as_a_finding() {
        let mut state = state_with(Vec::new());
        let reconciliation = Reconciliation {
            commits_since_session: vec!["fix: the thing this session was about".into()],
            uncommitted_changes: 7,
            ..Default::default()
        };
        let end = run(&mut state, &Ledgers::default(), &reconciliation);

        let labels: Vec<&str> = end.findings.iter().map(|f| f.kind.label()).collect();
        assert!(labels.contains(&"the repository moved on"), "{labels:?}");
        assert!(
            labels.contains(&"the working tree has changed"),
            "{labels:?}"
        );
        assert!(
            end.findings
                .iter()
                .any(|f| f.text.contains("7 uncommitted"))
        );
    }

    #[test]
    fn stale_claims_become_a_warning_with_a_proportion_not_just_a_count() {
        let mut state = state_with(vec![action("one", 1), action("two", 2)]);
        // Mark both stale, which is the shape a big `stale: 100` had.
        for item in state.items.iter_mut() {
            item.verified = Verification::Stale;
        }
        let end = run(&mut state, &Ledgers::default(), &Reconciliation::default());
        let finding = end
            .findings
            .iter()
            .find(|f| f.kind == FindingKind::StaleClaims)
            .expect("stale claims are a finding, not header metadata");
        assert!(finding.text.contains("2 of 2"), "{}", finding.text);
        assert!(finding.text.contains("Most"), "{}", finding.text);
    }
}
