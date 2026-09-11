//! S5 — repository reconciliation (spec §10.1).
//!
//! An artifact describes a session that has already ended; the repository has
//! moved on. Reconciliation compares the two and **annotates**: it never
//! deletes an item and never rewrites history. When they disagree, the
//! repository wins, because that is what the next agent will actually run
//! against.
//!
//! Every command here is read-only, on an explicit allowlist, and executed
//! through `std::process::Command` — never a shell. Session content is data;
//! nothing found in a transcript is ever executed.

use crate::ir::Session;
use crate::pipeline::fold::state::{FoldState, Verification};
use crate::pipeline::ledgers::Ledgers;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What reconciliation found.
#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Reconciliation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Branch recorded in the session, when it differs from the current one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_branch: Option<String>,
    /// Commit subjects created after the session ended.
    pub commits_since_session: Vec<String>,
    /// Ledger files that are no longer on disk.
    pub missing_files: Vec<String>,
    /// Files the session said it deleted that still exist.
    pub contradicted_files: Vec<String>,
    /// Files that changed after the session ended.
    pub changed_since_session: Vec<String>,
    /// Commits from the session that are no longer reachable (rebased?).
    pub missing_commits: Vec<String>,
    pub uncommitted_changes: usize,
    /// Why reconciliation was skipped or incomplete.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Reconciliation {
    /// Number of findings that contradict the artifact outright.
    pub fn contradictions(&self) -> usize {
        self.contradicted_files.len()
    }

    /// Number of findings that make the artifact merely out of date.
    pub fn stale(&self) -> usize {
        self.missing_files.len() + self.missing_commits.len() + self.changed_since_session.len()
    }

    /// True when the repository moved after the session.
    pub fn repo_moved(&self) -> bool {
        !self.commits_since_session.is_empty() || !self.changed_since_session.is_empty()
    }
}

/// Reconcile the ledgers and fold state against `repo`.
pub fn run(
    session: &Session,
    ledgers: &Ledgers,
    state: &mut FoldState,
    repo: &Path,
) -> Reconciliation {
    let mut report = Reconciliation {
        session_branch: session.meta.git_branch.clone(),
        ..Default::default()
    };

    let Some(toplevel) = git(repo, &["rev-parse", "--show-toplevel"]) else {
        report.note = Some(format!("{} is not a git repository", repo.display()));
        check_files(&mut report, ledgers, repo, None);
        annotate_items(state, &report, ledgers);
        return report;
    };
    let root = PathBuf::from(toplevel.trim());
    report.repo = Some(root.clone());
    report.head = git(&root, &["rev-parse", "HEAD"]).map(|head| head.trim().to_string());
    report.branch = git(&root, &["branch", "--show-current"])
        .map(|branch| branch.trim().to_string())
        .filter(|branch| !branch.is_empty());

    if let Some(status) = git(&root, &["status", "--porcelain=v1"]) {
        report.uncommitted_changes = status
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
    }

    // Commits landed after the session ended.
    if let Some(ended_at) = &session.meta.ended_at
        && let Some(log) = git(
            &root,
            &[
                "log",
                "--format=%h %s",
                &format!("--since={ended_at}"),
                "--max-count=10",
            ],
        )
    {
        report.commits_since_session = log
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_string)
            .collect();
    }

    // Commits the session made that are no longer reachable.
    for commit in &ledgers.git.commits {
        if git(
            &root,
            &["cat-file", "-e", &format!("{}^{{commit}}", commit.sha)],
        )
        .is_none()
        {
            report.missing_commits.push(commit.sha.clone());
        }
    }

    check_files(
        &mut report,
        ledgers,
        &root,
        session.meta.ended_at.as_deref(),
    );
    annotate_items(state, &report, ledgers);
    report
}

/// Compare ledger files against the working tree.
fn check_files(
    report: &mut Reconciliation,
    ledgers: &Ledgers,
    root: &Path,
    session_ended_at: Option<&str>,
) {
    for file in &ledgers.files {
        // An inferred path came from a shell heuristic; not evidence enough to
        // report as missing.
        if file.inferred {
            continue;
        }
        let path = resolve(root, &file.path);
        let exists = path.exists();
        if file.deleted && exists {
            report.contradicted_files.push(file.path.clone());
            continue;
        }
        if !file.deleted && !exists {
            report.missing_files.push(file.path.clone());
            continue;
        }
        if file.edits == 0 {
            continue;
        }
        if let Some(ended_at) = session_ended_at
            && changed_after(root, &file.path, ended_at)
        {
            report.changed_since_session.push(file.path.clone());
        }
    }
}

/// True when the file's last commit is newer than the session's end.
fn changed_after(root: &Path, path: &str, ended_at: &str) -> bool {
    git(
        root,
        &[
            "log",
            "--format=%H",
            "-1",
            &format!("--since={ended_at}"),
            "--",
            path,
        ],
    )
    .is_some_and(|out| !out.trim().is_empty())
}

fn resolve(root: &Path, path: &str) -> PathBuf {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    }
}

/// Mark items whose file or command claims no longer hold.
fn annotate_items(state: &mut FoldState, report: &Reconciliation, ledgers: &Ledgers) {
    // Paths that the repository confirms exist, for the "verified" case.
    let known: Vec<&str> = ledgers
        .files
        .iter()
        .filter(|file| {
            !report.missing_files.contains(&file.path)
                && !report.changed_since_session.contains(&file.path)
        })
        .map(|file| file.path.as_str())
        .collect();

    for item in &mut state.items {
        if !item.status.is_active() {
            continue;
        }
        let mentioned = mentioned_paths(&item.text);
        if mentioned.is_empty() {
            continue;
        }
        if mentioned.iter().any(|path| {
            report
                .contradicted_files
                .iter()
                .any(|file| file.contains(path.as_str()))
        }) {
            item.verified = Verification::Contradicted;
            continue;
        }
        let stale = mentioned.iter().any(|path| {
            report
                .missing_files
                .iter()
                .any(|file| file.contains(path.as_str()))
                || report
                    .changed_since_session
                    .iter()
                    .any(|file| file.contains(path.as_str()))
        });
        if stale {
            item.verified = Verification::Stale;
            item.confidence = crate::pipeline::fold::ops::Confidence::Low;
        } else if mentioned
            .iter()
            .any(|path| known.iter().any(|file| file.contains(path.as_str())))
        {
            item.verified = Verification::Verified;
        }
    }
}

/// Path-like tokens in item text (`src/auth.ts`, `` `packages/x/y.rs` ``).
fn mentioned_paths(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| word.trim_matches(|c: char| "`'\",;:()[]".contains(c)))
        .filter(|word| word.contains('/') && word.contains('.') && !word.contains("://"))
        .map(str::to_string)
        .collect()
}

/// The only git invocations sctxx makes (spec §10.1 allowlist).
const ALLOWED: &[&str] = &[
    "rev-parse",
    "branch",
    "status",
    "log",
    "cat-file",
    "check-ignore",
];

/// Whether git would ignore `path`.
///
/// `None` means "cannot tell" — not a work tree, git missing, or the command
/// failed. Only `Some(false)` is worth warning about.
///
/// This is the one check made outside extraction: an artifact carries real
/// session content, and `git add -A` in the user's project must not sweep it
/// up. Read-only, on the same allowlist as everything else here.
pub fn is_git_ignored(path: &Path) -> Option<bool> {
    if !ALLOWED.contains(&"check-ignore") {
        return None;
    }
    let status = Command::new("git")
        .args(["check-ignore", "--quiet", "--"])
        .arg(path)
        .current_dir(path)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0")
        .status()
        .ok()?;
    match status.code() {
        // 0 = ignored, 1 = not ignored, anything else = no answer.
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

/// Run a read-only git command, returning `None` on any failure.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let subcommand = args.first()?;
    // The allowlist is the guarantee, not a convention: anything not on it is
    // refused here rather than trusted to the call sites.
    if !ALLOWED.contains(subcommand) {
        return None;
    }
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        // Never let a user's git config open an editor or a pager.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{AgentKind, SessionMeta};
    use crate::pipeline::ledgers::{FileOp, FileRecord};

    fn session() -> Session {
        Session {
            agent: AgentKind::ClaudeCode,
            id: "s".into(),
            source_paths: vec![],
            source_hash: String::new(),
            meta: SessionMeta::default(),
            events: vec![],
            active: vec![],
            native_compactions: vec![],
            diagnostics: vec![],
        }
    }

    fn file(path: &str, deleted: bool, edits: u32) -> FileRecord {
        FileRecord {
            path: path.into(),
            ops: vec![FileOp::Edit],
            created: false,
            deleted,
            moved_to: None,
            edits,
            reads: 0,
            first_evt: 0,
            last_evt: 1,
            last_edit_succeeded: Some(true),
            inferred: false,
        }
    }

    #[test]
    fn only_allowlisted_git_subcommands_run() {
        let dir = tempfile::tempdir().expect("tempdir");
        // `init` would succeed as a git command but is not read-only.
        assert!(git(dir.path(), &["init"]).is_none());
    }

    #[test]
    fn a_non_repository_is_reported_not_fatal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut state = FoldState::new();
        let report = run(&session(), &Ledgers::default(), &mut state, dir.path());
        assert!(report.note.is_some(), "{report:?}");
        assert!(report.repo.is_none());
    }

    #[test]
    fn a_missing_file_is_stale_and_a_surviving_deleted_file_is_contradicted() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("kept.rs"), "fn main() {}").expect("write");
        let mut ledgers = Ledgers::default();
        ledgers.files.push(file("gone.rs", false, 1));
        ledgers.files.push(file("kept.rs", true, 1));

        let mut report = Reconciliation::default();
        check_files(&mut report, &ledgers, dir.path(), None);
        assert_eq!(report.missing_files, vec!["gone.rs".to_string()]);
        assert_eq!(report.contradicted_files, vec!["kept.rs".to_string()]);
        assert_eq!(report.stale(), 1);
        assert_eq!(report.contradictions(), 1);
    }

    #[test]
    fn an_inferred_path_is_never_reported_as_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut ledgers = Ledgers::default();
        let mut inferred = file("maybe/gone.rs", false, 0);
        inferred.inferred = true;
        ledgers.files.push(inferred);
        let mut report = Reconciliation::default();
        check_files(&mut report, &ledgers, dir.path(), None);
        assert!(report.missing_files.is_empty());
    }

    #[test]
    fn items_are_annotated_never_removed() {
        use crate::pipeline::fold::ops::{Confidence, EvtRange, ItemKind, NewItem, Op};
        let mut state = FoldState::new();
        state.apply(
            "c0",
            &Op::Add {
                item: NewItem {
                    kind: ItemKind::NextAction,
                    text: "fix src/host/module-host.ts".into(),
                    why: None,
                    quote: None,
                    rejected: vec![],
                    sources: vec![EvtRange::new(0, 1)],
                    confidence: Confidence::High,
                },
            },
        );
        let mut ledgers = Ledgers::default();
        ledgers
            .files
            .push(file("src/host/module-host.ts", false, 3));
        let report = Reconciliation {
            missing_files: vec!["src/host/module-host.ts".to_string()],
            ..Reconciliation::default()
        };
        annotate_items(&mut state, &report, &ledgers);
        let item = state.get("N1").expect("item survives");
        assert_eq!(item.verified, Verification::Stale);
        assert_eq!(item.confidence, Confidence::Low);
        assert_eq!(state.active().len(), 1, "reconciliation removed an item");
    }

    #[test]
    fn path_extraction_ignores_urls_and_prose() {
        assert_eq!(
            mentioned_paths("edit `src/a.rs` next"),
            vec!["src/a.rs".to_string()]
        );
        assert!(mentioned_paths("see https://example.com/a.html").is_empty());
        assert!(mentioned_paths("nothing path-like here").is_empty());
    }
}
