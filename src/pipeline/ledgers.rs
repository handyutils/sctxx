//! S1 — deterministic ledgers (spec §7.2).
//!
//! Everything an artifact says about files, commands, errors, plans, and git is
//! computed here in Rust, not asked of a model. That is what makes the
//! `--llm none` artifact useful on its own and what keeps the LLM stages
//! honest: the fold sees the same ledger slice a reviewer can recompute.

use crate::adapters::tools::{COMMAND_ARG_KEYS, PATH_ARG_KEYS};
use crate::ir::{EventIdx, EventKind, PlanItem, Session, ToolClass};
use crate::vendor::codex::apply_patch_paths::{PatchOp, parse_ops};
use crate::vendor::codex::secrets::{RedactMode, redact};
use crate::vendor::codex::truncate::{
    truncate_head_bytes, truncate_middle_tokens, truncate_tail_bytes,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// What happened to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOp {
    Read,
    Create,
    Edit,
    Delete,
    Move,
}

/// Everything the session did to one path.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FileRecord {
    pub path: String,
    pub ops: Vec<FileOp>,
    pub created: bool,
    pub deleted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moved_to: Option<String>,
    pub edits: u32,
    pub reads: u32,
    pub first_evt: EventIdx,
    pub last_evt: EventIdx,
    /// `None` when the provider gave no success signal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_edit_succeeded: Option<bool>,
    /// True when the path came from a shell heuristic rather than an edit tool.
    pub inferred: bool,
}

/// Coarse command category, used to report a last-known state per kind of work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CmdCategory {
    Test,
    Build,
    Lint,
    Git,
    PackageManager,
    Run,
    #[default]
    Other,
}

impl CmdCategory {
    pub fn label(self) -> &'static str {
        match self {
            CmdCategory::Test => "test",
            CmdCategory::Build => "build",
            CmdCategory::Lint => "lint",
            CmdCategory::Git => "git",
            CmdCategory::PackageManager => "package manager",
            CmdCategory::Run => "run",
            CmdCategory::Other => "other",
        }
    }
}

/// One command execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CommandRecord {
    pub evt: EventIdx,
    pub command: String,
    pub normalized: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    pub category: CmdCategory,
    pub output_head: String,
    pub output_tail: String,
}

impl CommandRecord {
    /// Whether this run should be read as a failure.
    pub fn failed(&self) -> bool {
        self.is_error == Some(true) || self.exit_code.is_some_and(|code| code != 0)
    }

    /// Human-readable status for the artifact.
    pub fn status(&self) -> &'static str {
        match (self.failed(), self.exit_code, self.is_error) {
            (true, _, _) => "FAILED",
            (false, Some(0), _) | (false, _, Some(false)) => "ok",
            _ => "unknown",
        }
    }
}

/// Whether an error signature was still live at the end of the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum ErrorStatus {
    Resolved {
        evt: EventIdx,
    },
    Unresolved,
    #[default]
    Unknown,
}

/// A distinct failure, deduplicated by normalized signature.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ErrorRecord {
    /// First 12 hex characters of the signature hash.
    pub sig: String,
    pub example: String,
    pub first_evt: EventIdx,
    pub last_evt: EventIdx,
    pub occurrences: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub status: ErrorStatus,
}

/// A human message, redacted and truncated.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UserMessageRecord {
    pub evt: EventIdx,
    pub text: String,
}

/// A commit observed in the session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CommitRecord {
    pub evt: EventIdx,
    pub sha: String,
    pub subject: String,
}

/// Git activity recovered from command output.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GitLedger {
    pub commits: Vec<CommitRecord>,
    pub branches: Vec<String>,
    pub pushed: bool,
    pub pull_requests: Vec<String>,
}

/// The last plan state the agent published.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanLedger {
    pub evt: EventIdx,
    pub items: Vec<PlanItem>,
}

/// All deterministic ledgers for one session.
///
/// Deserializable so `sctxx verify` can reconcile an artifact that was written
/// days ago without re-parsing the session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Ledgers {
    pub files: Vec<FileRecord>,
    pub commands: Vec<CommandRecord>,
    pub errors: Vec<ErrorRecord>,
    pub user_messages: Vec<UserMessageRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanLedger>,
    pub git: GitLedger,
    /// Provider compaction summaries, in event order. Low trust.
    pub prior_summaries: Vec<UserMessageRecord>,
}

impl Ledgers {
    /// The last run of each command category, newest first.
    pub fn last_command_status(&self) -> Vec<&CommandRecord> {
        let mut last: BTreeMap<(CmdCategory, &str), &CommandRecord> = BTreeMap::new();
        for record in &self.commands {
            last.insert((record.category, record.normalized.as_str()), record);
        }
        let mut out: Vec<&CommandRecord> = last.into_values().collect();
        out.sort_by_key(|record| std::cmp::Reverse(record.evt));
        out
    }

    /// Error signatures still unresolved at the end of the session.
    pub fn unresolved_errors(&self) -> Vec<&ErrorRecord> {
        let mut out: Vec<&ErrorRecord> = self
            .errors
            .iter()
            .filter(|e| e.status == ErrorStatus::Unresolved)
            .collect();
        out.sort_by(|a, b| {
            b.occurrences
                .cmp(&a.occurrences)
                .then_with(|| a.sig.cmp(&b.sig))
        });
        out
    }

    /// Repeatedly unresolved failures: deterministic dead-end candidates that
    /// the fold is told about rather than asked to discover (spec §7.2).
    pub fn dead_end_candidates(&self) -> Vec<&ErrorRecord> {
        self.unresolved_errors()
            .into_iter()
            .filter(|e| e.occurrences >= 3)
            .collect()
    }

    /// Files that were edited, most recently touched first.
    pub fn edited_files(&self) -> Vec<&FileRecord> {
        let mut out: Vec<&FileRecord> = self.files.iter().filter(|f| f.edits > 0).collect();
        out.sort_by_key(|record| std::cmp::Reverse(record.last_evt));
        out
    }
}

/// How much of a command's output the ledger keeps.
const OUTPUT_HEAD_BYTES: usize = 400;
const OUTPUT_TAIL_BYTES: usize = 800;
/// Human messages are the highest-signal evidence; they are kept nearly whole.
const USER_MESSAGE_TOKENS: usize = 1_500;
const PRIOR_SUMMARY_TOKENS: usize = 1_500;

/// Compute every ledger over the session's active branch.
pub fn build(session: &Session, redact_mode: RedactMode) -> Ledgers {
    let mut ledgers = Ledgers::default();
    let mut files: BTreeMap<String, FileRecord> = BTreeMap::new();
    // call_id -> (evt, command text) awaiting its result.
    let mut pending_shell: BTreeMap<String, (EventIdx, String)> = BTreeMap::new();
    // call_id -> (evt, path) for edit calls awaiting their result.
    let mut pending_edit: BTreeMap<String, (EventIdx, Vec<String>)> = BTreeMap::new();

    for event in session.active_events() {
        match &event.kind {
            EventKind::UserMessage {
                text,
                is_meta: false,
            } => {
                ledgers.user_messages.push(UserMessageRecord {
                    evt: event.idx,
                    text: redact(
                        &truncate_middle_tokens(text, USER_MESSAGE_TOKENS),
                        redact_mode,
                    ),
                });
            }
            EventKind::UserAnswer { question, answer } => {
                ledgers.user_messages.push(UserMessageRecord {
                    evt: event.idx,
                    text: redact(
                        &truncate_middle_tokens(
                            &format!("(asked: {question}) {answer}"),
                            USER_MESSAGE_TOKENS,
                        ),
                        redact_mode,
                    ),
                });
            }
            EventKind::NativeCompactionSummary { text } | EventKind::BranchSummary { text } => {
                ledgers.prior_summaries.push(UserMessageRecord {
                    evt: event.idx,
                    text: redact(
                        &truncate_middle_tokens(text, PRIOR_SUMMARY_TOKENS),
                        redact_mode,
                    ),
                });
            }
            EventKind::PlanUpdate { items } => {
                ledgers.plan = Some(PlanLedger {
                    evt: event.idx,
                    items: items.clone(),
                });
            }
            EventKind::ToolCall {
                call_id,
                class,
                args,
                ..
            } => {
                match class {
                    ToolClass::Shell => {
                        if let Some(command) = command_of(args) {
                            pending_shell.insert(call_id.clone(), (event.idx, command.clone()));
                            for (path, op) in shell_file_ops(&command) {
                                record_file(&mut files, &path, op, event.idx, None, true);
                            }
                        }
                    }
                    ToolClass::Edit => {
                        let paths = edit_paths(args);
                        for path in &paths {
                            record_file(&mut files, path, FileOp::Edit, event.idx, None, false);
                        }
                        pending_edit.insert(call_id.clone(), (event.idx, paths));
                    }
                    ToolClass::Read => {
                        if let Some(path) = path_of(args) {
                            record_file(&mut files, &path, FileOp::Read, event.idx, None, false);
                        }
                    }
                    _ => {}
                }
                // A patch envelope describes creates, deletes, and moves.
                if *class == ToolClass::Edit {
                    for op in patch_ops(args) {
                        apply_patch_op(&mut files, &op, event.idx);
                    }
                }
            }
            EventKind::ToolResult {
                call_id,
                output,
                is_error,
                exit_code,
            } => {
                if let Some((call_evt, command)) = pending_shell.remove(call_id) {
                    ledgers.commands.push(command_record(
                        call_evt,
                        command,
                        output,
                        *is_error,
                        *exit_code,
                        redact_mode,
                    ));
                }
                if let Some((_, paths)) = pending_edit.remove(call_id) {
                    let succeeded = match is_error {
                        Some(flag) => Some(!flag),
                        None => Some(!looks_like_error(output)),
                    };
                    for path in paths {
                        if let Some(record) = files.get_mut(&path) {
                            record.last_edit_succeeded = succeeded;
                        }
                    }
                }
                collect_error(
                    &mut ledgers,
                    event.idx,
                    output,
                    *is_error,
                    *exit_code,
                    None,
                    redact_mode,
                );
            }
            EventKind::ShellExecution {
                command,
                output,
                exit_code,
            } => {
                ledgers.commands.push(command_record(
                    event.idx,
                    command.clone(),
                    output,
                    None,
                    *exit_code,
                    redact_mode,
                ));
                for (path, op) in shell_file_ops(command) {
                    record_file(&mut files, &path, op, event.idx, None, true);
                }
                collect_error(
                    &mut ledgers,
                    event.idx,
                    output,
                    None,
                    *exit_code,
                    Some(normalize_command(command)),
                    redact_mode,
                );
            }
            _ => {}
        }
    }

    // Attribute each error to the command that produced it, when known.
    attribute_commands(&mut ledgers);
    resolve_error_statuses(&mut ledgers);
    read_git_ledger(&mut ledgers);

    ledgers.files = files.into_values().collect();
    ledgers
}

fn command_record(
    evt: EventIdx,
    command: String,
    output: &str,
    is_error: Option<bool>,
    exit_code: Option<i32>,
    redact_mode: RedactMode,
) -> CommandRecord {
    let exit_code = exit_code.or_else(|| exit_code_in_output(output));
    CommandRecord {
        evt,
        normalized: normalize_command(&command),
        category: categorize(&command),
        command: redact(&command, redact_mode),
        exit_code,
        is_error,
        output_head: redact(&truncate_head_bytes(output, OUTPUT_HEAD_BYTES), redact_mode),
        output_tail: redact(&truncate_tail_bytes(output, OUTPUT_TAIL_BYTES), redact_mode),
    }
}

/// Normalize a command so repeated runs collapse to one identity.
pub fn normalize_command(command: &str) -> String {
    let mut text = command.trim().to_string();
    // Strip `cd <dir> &&` prefixes.
    while let Some(rest) = text.strip_prefix("cd ") {
        match rest.split_once("&&") {
            Some((_, tail)) => text = tail.trim().to_string(),
            None => break,
        }
    }
    // Redact env assignment values but keep the variable names.
    let mut parts: Vec<String> = Vec::new();
    for token in text.split_whitespace() {
        if parts.is_empty()
            && let Some((name, _)) = token.split_once('=')
            && !name.is_empty()
            && name.chars().all(|c| c.is_ascii_uppercase() || c == '_')
        {
            parts.push(format!("{name}=***"));
            continue;
        }
        parts.push(token.to_string());
    }
    parts.join(" ")
}

/// Classify a command by its leading tokens.
pub fn categorize(command: &str) -> CmdCategory {
    let normalized = normalize_command(command);
    let lower = normalized.to_ascii_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let first = words.first().copied().unwrap_or("");
    let second = words.get(1).copied().unwrap_or("");

    let is_test = matches!(second, "test" | "tests") && !lower.contains("--no-run")
        || matches!(first, "pytest" | "vitest" | "jest" | "phpunit" | "rspec")
        || matches!((first, second), ("go", "test") | ("cargo", "nextest"))
        || lower.contains(" vitest ")
        || lower.ends_with(" vitest");
    if is_test {
        return CmdCategory::Test;
    }
    if matches!(second, "build" | "compile" | "check")
        || matches!(
            first,
            "make" | "tsc" | "webpack" | "vite" | "cmake" | "bazel"
        )
    {
        return CmdCategory::Build;
    }
    if matches!(second, "clippy" | "lint" | "fmt" | "format")
        || matches!(
            first,
            "eslint" | "ruff" | "prettier" | "black" | "shellcheck" | "biome"
        )
    {
        return CmdCategory::Lint;
    }
    if first == "git" {
        return CmdCategory::Git;
    }
    if matches!(
        first,
        "npm" | "pnpm" | "yarn" | "bun" | "cargo" | "pip" | "uv" | "poetry" | "brew"
    ) {
        return CmdCategory::PackageManager;
    }
    if matches!(
        first,
        "node" | "python" | "python3" | "deno" | "ruby" | "bash" | "sh"
    ) {
        return CmdCategory::Run;
    }
    CmdCategory::Other
}

fn command_of(args: &Value) -> Option<String> {
    for key in COMMAND_ARG_KEYS {
        match args.get(key) {
            Some(Value::String(text)) if !text.is_empty() => return Some(text.clone()),
            // Codex writes `shell` commands as an argv array.
            Some(Value::Array(items)) => {
                let joined: Vec<String> = items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                if !joined.is_empty() {
                    return Some(joined.join(" "));
                }
            }
            _ => {}
        }
    }
    if let Value::String(text) = args {
        return Some(text.clone());
    }
    None
}

fn path_of(args: &Value) -> Option<String> {
    for key in PATH_ARG_KEYS {
        if let Some(Value::String(text)) = args.get(key)
            && !text.is_empty()
        {
            return Some(text.clone());
        }
    }
    None
}

/// Every path an edit tool call touched.
fn edit_paths(args: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some(path) = path_of(args) {
        paths.push(path);
    }
    // Claude's MultiEdit takes a list of edits, each with its own path.
    if let Some(edits) = args.get("edits").and_then(Value::as_array) {
        for edit in edits {
            if let Some(path) = path_of(edit) {
                paths.push(path);
            }
        }
    }
    for op in patch_ops(args) {
        paths.push(op.path().to_string_lossy().into_owned());
    }
    paths.sort();
    paths.dedup();
    paths
}

/// Parse a Codex `apply_patch` envelope out of whatever shape it arrived in.
fn patch_ops(args: &Value) -> Vec<PatchOp> {
    let candidates = [
        args.get("input").and_then(Value::as_str),
        args.get("patch").and_then(Value::as_str),
        args.as_str(),
    ];
    for candidate in candidates.into_iter().flatten() {
        let ops = parse_ops(candidate);
        if !ops.is_empty() {
            return ops;
        }
    }
    Vec::new()
}

fn apply_patch_op(files: &mut BTreeMap<String, FileRecord>, op: &PatchOp, evt: EventIdx) {
    let path = op.path().to_string_lossy().into_owned();
    match op {
        PatchOp::Add(_) => record_file(files, &path, FileOp::Create, evt, None, false),
        PatchOp::Update(_) => record_file(files, &path, FileOp::Edit, evt, None, false),
        PatchOp::Delete(_) => record_file(files, &path, FileOp::Delete, evt, None, false),
        PatchOp::Move { to, .. } => record_file(
            files,
            &path,
            FileOp::Move,
            evt,
            Some(to.to_string_lossy().into_owned()),
            false,
        ),
    }
}

/// Low-confidence file operations inferred from a shell command. Marked
/// `inferred` so the artifact never states them as fact (spec §7.2).
fn shell_file_ops(command: &str) -> Vec<(String, FileOp)> {
    let mut ops = Vec::new();
    for segment in command.split([';', '\n']).flat_map(|s| s.split("&&")) {
        let words: Vec<&str> = segment.split_whitespace().collect();
        let Some(first) = words.first() else { continue };
        let (verb, rest): (&str, &[&str]) = match *first {
            "git" => (
                words.get(1).copied().unwrap_or(""),
                &words[2.min(words.len())..],
            ),
            other => (other, &words[1.min(words.len())..]),
        };
        let paths: Vec<String> = rest
            .iter()
            .filter(|word| !word.starts_with('-'))
            .map(|word| word.trim_matches(['"', '\'']).to_string())
            .filter(|word| !word.is_empty())
            .collect();
        match verb {
            "rm" => ops.extend(paths.into_iter().map(|path| (path, FileOp::Delete))),
            "mv" => {
                if let Some(source) = paths.first() {
                    ops.push((source.clone(), FileOp::Move));
                }
                if let Some(target) = paths.last().filter(|_| paths.len() > 1) {
                    ops.push((target.clone(), FileOp::Create));
                }
            }
            "cp" | "touch" => {
                if let Some(target) = paths.last() {
                    ops.push((target.clone(), FileOp::Create));
                }
            }
            _ => {}
        }
        // Output redirection creates or rewrites a file.
        if let Some(target) = segment.split('>').nth(1)
            && let Some(word) = target.split_whitespace().next()
            && !word.starts_with('&')
            && word.contains('.')
        {
            ops.push((word.trim_matches(['"', '\'']).to_string(), FileOp::Create));
        }
    }
    ops
}

fn record_file(
    files: &mut BTreeMap<String, FileRecord>,
    path: &str,
    op: FileOp,
    evt: EventIdx,
    moved_to: Option<String>,
    inferred: bool,
) {
    if path.is_empty() {
        return;
    }
    let record = files.entry(path.to_string()).or_insert_with(|| FileRecord {
        path: path.to_string(),
        ops: Vec::new(),
        created: false,
        deleted: false,
        moved_to: None,
        edits: 0,
        reads: 0,
        first_evt: evt,
        last_evt: evt,
        last_edit_succeeded: None,
        inferred,
    });
    record.ops.push(op);
    record.last_evt = record.last_evt.max(evt);
    record.first_evt = record.first_evt.min(evt);
    if !inferred {
        record.inferred = false;
    }
    match op {
        FileOp::Read => record.reads += 1,
        FileOp::Edit => record.edits += 1,
        FileOp::Create => {
            record.created = true;
            record.edits += 1;
        }
        // A delete inferred from a shell command is not enough to claim the
        // file is gone; reconciliation against the repo decides (spec §7.2).
        FileOp::Delete => {
            if !inferred {
                record.deleted = true;
            }
        }
        FileOp::Move => {
            if moved_to.is_some() {
                record.moved_to = moved_to;
            }
            record.edits += 1;
        }
    }
}

// ---- errors -----------------------------------------------------------------

const ERROR_MARKERS: &[&str] = &[
    "error[",
    "error:",
    "Error:",
    "ERROR",
    "Traceback",
    "panicked at",
    "FAILED",
    "FAIL ",
    "✕",
    "npm ERR!",
    "Exception",
    "error TS",
    "AssertionError",
    "SyntaxError",
    "TypeError",
];

/// True when output looks like a failure even without an exit code.
pub fn looks_like_error(output: &str) -> bool {
    ERROR_MARKERS.iter().any(|marker| output.contains(marker))
}

fn exit_code_in_output(output: &str) -> Option<i32> {
    // Codex renders shell results with a leading metadata line.
    for line in output.lines().take(3) {
        if let Some(rest) = line.split("exit code:").nth(1)
            && let Some(number) = rest.split_whitespace().next()
            && let Ok(code) = number.trim_matches(['}', ',', ')']).parse::<i32>()
        {
            return Some(code);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn collect_error(
    ledgers: &mut Ledgers,
    evt: EventIdx,
    output: &str,
    is_error: Option<bool>,
    exit_code: Option<i32>,
    command: Option<String>,
    redact_mode: RedactMode,
) {
    let failed = is_error == Some(true)
        || exit_code.is_some_and(|code| code != 0)
        || exit_code_in_output(output).is_some_and(|code| code != 0);
    if !failed && !looks_like_error(output) {
        return;
    }
    let Some(snippet) = error_snippet(output) else {
        return;
    };
    let signature = normalize_signature(&snippet);
    let sig = signature_id(&signature);
    let example = redact(&truncate_head_bytes(&snippet, 300), redact_mode);

    if let Some(existing) = ledgers.errors.iter_mut().find(|record| record.sig == sig) {
        existing.occurrences += 1;
        existing.last_evt = evt;
        if existing.command.is_none() {
            existing.command = command;
        }
        return;
    }
    ledgers.errors.push(ErrorRecord {
        sig,
        example,
        first_evt: evt,
        last_evt: evt,
        occurrences: 1,
        command,
        status: ErrorStatus::Unknown,
    });
}

/// The first error line plus up to two following lines.
fn error_snippet(output: &str) -> Option<String> {
    let lines: Vec<&str> = output.lines().collect();
    let start = lines.iter().position(|line| looks_like_error(line))?;
    let end = (start + 3).min(lines.len());
    let snippet = lines[start..end].join("\n");
    (!snippet.trim().is_empty()).then_some(snippet)
}

/// Erase everything run-specific so the same failure hashes the same.
pub fn normalize_signature(snippet: &str) -> String {
    let mut out = String::with_capacity(snippet.len());
    let mut chars = snippet.chars().peekable();
    let mut token = String::new();

    let flush = |token: &mut String, out: &mut String| {
        if token.is_empty() {
            return;
        }
        let replacement = if token.len() >= 7 && token.chars().all(|c| c.is_ascii_hexdigit()) {
            "<hex>"
        } else if token.chars().all(|c| c.is_ascii_digit()) {
            "<n>"
        } else if token.contains('/') && token.len() > 1 {
            "<path>"
        } else if is_uuid(token) {
            "<uuid>"
        } else if is_duration(token) {
            "<dur>"
        } else {
            token.as_str()
        };
        out.push_str(replacement);
        token.clear();
    };

    while let Some(ch) = chars.next() {
        if ch.is_whitespace() {
            flush(&mut token, &mut out);
            // Collapse runs of whitespace to one space.
            while chars.peek().is_some_and(|next| next.is_whitespace()) {
                chars.next();
            }
            out.push(' ');
        } else if ch.is_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | ':' | '\\') {
            token.push(ch);
        } else {
            flush(&mut token, &mut out);
            out.push(ch);
        }
    }
    flush(&mut token, &mut out);
    out.trim().to_string()
}

fn is_uuid(token: &str) -> bool {
    let groups: Vec<&str> = token.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .all(|group| group.chars().all(|c| c.is_ascii_hexdigit()))
        && groups[0].len() == 8
}

fn is_duration(token: &str) -> bool {
    let trimmed = token.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    !trimmed.is_empty()
        && trimmed != token
        && trimmed.chars().all(|c| c.is_ascii_digit() || c == '.')
}

fn signature_id(signature: &str) -> String {
    let digest = Sha256::digest(signature.as_bytes());
    format!("{digest:x}").chars().take(12).collect()
}

/// Attach the nearest preceding command to each error signature.
fn attribute_commands(ledgers: &mut Ledgers) {
    for error in &mut ledgers.errors {
        if error.command.is_some() {
            continue;
        }
        error.command = ledgers
            .commands
            .iter()
            .filter(|command| command.evt <= error.first_evt)
            .max_by_key(|command| command.evt)
            .map(|command| command.normalized.clone());
    }
}

/// An error is resolved when the command that produced it later succeeded
/// without the signature; unresolved when its last run still failed.
fn resolve_error_statuses(ledgers: &mut Ledgers) {
    let commands = ledgers.commands.clone();
    for error in &mut ledgers.errors {
        let Some(command) = &error.command else {
            error.status = ErrorStatus::Unknown;
            continue;
        };
        let later_runs: Vec<&CommandRecord> = commands
            .iter()
            .filter(|record| &record.normalized == command && record.evt > error.last_evt)
            .collect();
        error.status = match later_runs.last() {
            Some(last) if !last.failed() => ErrorStatus::Resolved { evt: last.evt },
            Some(_) => ErrorStatus::Unresolved,
            None => {
                // No rerun: the failure is the last word on that command.
                let last_run = commands
                    .iter()
                    .filter(|record| &record.normalized == command)
                    .max_by_key(|record| record.evt);
                match last_run {
                    Some(run) if run.failed() => ErrorStatus::Unresolved,
                    Some(_) => ErrorStatus::Unknown,
                    None => ErrorStatus::Unresolved,
                }
            }
        };
    }
}

// ---- git --------------------------------------------------------------------

/// Recover commits, branch switches, pushes, and PR links from command output.
fn read_git_ledger(ledgers: &mut Ledgers) {
    let commands = ledgers.commands.clone();
    for record in &commands {
        if record.category != CmdCategory::Git {
            continue;
        }
        let output = format!("{}\n{}", record.output_head, record.output_tail);
        if record.normalized.contains("commit")
            && !record.failed()
            && let Some((sha, subject)) = parse_commit_line(&output)
        {
            ledgers.git.commits.push(CommitRecord {
                evt: record.evt,
                sha,
                subject,
            });
        }
        if record.normalized.contains("push") && !record.failed() {
            ledgers.git.pushed = true;
        }
        if let Some(branch) = parse_checkout_branch(&record.normalized)
            && !ledgers.git.branches.contains(&branch)
        {
            ledgers.git.branches.push(branch);
        }
        for url in parse_pr_urls(&output) {
            if !ledgers.git.pull_requests.contains(&url) {
                ledgers.git.pull_requests.push(url);
            }
        }
    }
    // A PR link can also appear in non-git command output (e.g. `gh pr create`).
    for record in &commands {
        let output = format!("{}\n{}", record.output_head, record.output_tail);
        for url in parse_pr_urls(&output) {
            if !ledgers.git.pull_requests.contains(&url) {
                ledgers.git.pull_requests.push(url);
            }
        }
    }
}

/// `git commit` prints `[branch abc1234] subject`.
fn parse_commit_line(output: &str) -> Option<(String, String)> {
    for line in output.lines() {
        let Some((head, subject)) = line
            .trim()
            .strip_prefix('[')
            .and_then(|rest| rest.split_once(']'))
        else {
            continue;
        };
        // `[branch abc1234]`, or `[branch (root-commit) abc1234]`.
        let mut parts = head.split_whitespace();
        let Some(_branch) = parts.next() else {
            continue;
        };
        let Some(sha) = parts.next_back() else {
            continue;
        };
        if sha.len() >= 7 && sha.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some((sha.to_string(), subject.trim().to_string()));
        }
    }
    None
}

fn parse_checkout_branch(normalized: &str) -> Option<String> {
    let words: Vec<&str> = normalized.split_whitespace().collect();
    let position = words
        .iter()
        .position(|word| matches!(*word, "checkout" | "switch"))?;
    words
        .get(position + 1..)?
        .iter()
        .find(|word| !word.starts_with('-'))
        .map(|word| word.to_string())
}

fn parse_pr_urls(output: &str) -> Vec<String> {
    output
        .split_whitespace()
        .filter(|word| word.starts_with("https://") && word.contains("/pull/"))
        .map(|word| word.trim_end_matches(['.', ',', ')']).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_normalize_to_one_identity() {
        assert_eq!(normalize_command("cd /repo && cargo test"), "cargo test");
        assert_eq!(normalize_command("  cargo   test  "), "cargo test");
        assert_eq!(
            normalize_command("TOKEN=abc123 cargo test"),
            "TOKEN=*** cargo test"
        );
    }

    #[test]
    fn categories_recognize_the_common_toolchains() {
        assert_eq!(categorize("cargo test --all-features"), CmdCategory::Test);
        assert_eq!(categorize("pnpm vitest run packages/x"), CmdCategory::Test);
        assert_eq!(categorize("pytest -k auth"), CmdCategory::Test);
        assert_eq!(categorize("cargo clippy -- -D warnings"), CmdCategory::Lint);
        assert_eq!(categorize("make build"), CmdCategory::Build);
        assert_eq!(categorize("git status"), CmdCategory::Git);
        assert_eq!(categorize("npm install"), CmdCategory::PackageManager);
        assert_eq!(categorize("ls -la"), CmdCategory::Other);
    }

    #[test]
    fn error_signatures_ignore_run_specific_detail() {
        let first = normalize_signature("error: /home/a/src/x.rs:12:4 failed after 1.3s");
        let second = normalize_signature("error: /home/b/src/x.rs:99:1 failed after 9.7s");
        assert_eq!(first, second, "{first} != {second}");
        assert_eq!(signature_id(&first).len(), 12);
    }

    #[test]
    fn error_detection_reads_markers_and_exit_codes() {
        assert!(looks_like_error("error[E0308]: mismatched types"));
        assert!(looks_like_error("npm ERR! missing script"));
        assert!(!looks_like_error("test result: ok. 12 passed"));
        assert_eq!(exit_code_in_output("exit code: 2\nboom"), Some(2));
    }

    #[test]
    fn shell_heuristics_are_conservative() {
        assert_eq!(
            shell_file_ops("rm -rf build/x.js"),
            vec![("build/x.js".into(), FileOp::Delete)]
        );
        assert!(shell_file_ops("cargo test").is_empty());
        let ops = shell_file_ops("echo hi > notes.txt");
        assert!(
            ops.contains(&("notes.txt".to_string(), FileOp::Create)),
            "{ops:?}"
        );
    }

    #[test]
    fn an_inferred_delete_never_marks_a_file_deleted() {
        let mut files = BTreeMap::new();
        record_file(&mut files, "a.rs", FileOp::Delete, 1, None, true);
        assert!(!files["a.rs"].deleted);
        record_file(&mut files, "a.rs", FileOp::Delete, 2, None, false);
        assert!(files["a.rs"].deleted);
        assert!(!files["a.rs"].inferred);
    }

    #[test]
    fn commit_lines_and_pr_urls_are_recovered() {
        assert_eq!(
            parse_commit_line("[main 4be91c2] fix lint"),
            Some(("4be91c2".to_string(), "fix lint".to_string()))
        );
        assert_eq!(parse_commit_line("nothing here"), None);
        assert_eq!(
            parse_pr_urls("created https://github.com/o/r/pull/12."),
            vec!["https://github.com/o/r/pull/12".to_string()]
        );
        assert_eq!(
            parse_checkout_branch("git checkout -b feat/x"),
            Some("feat/x".to_string())
        );
    }
}
