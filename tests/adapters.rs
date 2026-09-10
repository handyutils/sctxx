//! Adapter golden tests: every fixture's IR is snapshotted.
//!
//! These are the contract with the providers. A format change shows up as a
//! reviewable snapshot diff instead of a silent behavior change, which is the
//! whole reason the fixture corpus exists.

use sctxx::adapters::{self, source};
use sctxx::ir::{AgentKind, Diagnostic, EventKind, Session};
use std::path::{Path, PathBuf};

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(relative)
}

fn parse(relative: &str) -> Session {
    let path = fixture(relative);
    adapters::parse_path(&path, 0.5).unwrap_or_else(|error| panic!("{relative}: {error}"))
}

/// A compact, reviewable projection of the IR. The full event list would make
/// every snapshot diff unreadable; this keeps the parts a reviewer checks.
fn summarize(session: &Session) -> serde_json::Value {
    let active: Vec<serde_json::Value> = session
        .active_events()
        .map(|event| {
            serde_json::json!({
                "evt": event.idx,
                "stream": event.stream,
                "kind": event.kind.name(),
                "detail": detail(&event.kind),
            })
        })
        .collect();
    serde_json::json!({
        "agent": session.agent,
        "id": session.id,
        "meta": session.meta,
        "events": session.events.len(),
        "active_indices": session.active,
        "user_turns": session.user_turns(),
        "kind_counts": session.kind_counts(),
        "native_compactions": session.native_compactions,
        "diagnostics": session.diagnostics,
        "active": active,
    })
}

/// One line of evidence per event: enough to see what was parsed, short enough
/// to read in a diff.
fn detail(kind: &EventKind) -> String {
    let text = match kind {
        EventKind::UserMessage { text, is_meta } => {
            format!("{}{text}", if *is_meta { "[harness] " } else { "" })
        }
        EventKind::AssistantText { text, .. } => text.clone(),
        EventKind::Reasoning { text, redacted } => text
            .clone()
            .unwrap_or_else(|| format!("<redacted={redacted}>")),
        EventKind::ToolCall {
            name, class, args, ..
        } => {
            format!("{name} ({class:?}) {args}")
        }
        EventKind::ToolResult {
            output,
            is_error,
            exit_code,
            ..
        } => {
            format!("error={is_error:?} exit={exit_code:?} {output}")
        }
        EventKind::ShellExecution {
            command, exit_code, ..
        } => {
            format!("{command} -> exit={exit_code:?}")
        }
        EventKind::PlanUpdate { items } => items
            .iter()
            .map(|item| format!("[{}] {}", item.status, item.text))
            .collect::<Vec<String>>()
            .join(" | "),
        EventKind::UserAnswer { question, answer } => format!("Q: {question} / A: {answer}"),
        EventKind::NativeCompactionSummary { text } | EventKind::BranchSummary { text } => {
            text.clone()
        }
        EventKind::Rollback { num_turns } => format!("{num_turns} turns"),
        EventKind::ModelChange { model } => model.clone(),
        EventKind::SubagentSpawn { stream, prompt } => format!("stream {stream}: {prompt}"),
        EventKind::SubagentResult { stream, text } => format!("stream {stream}: {text}"),
        EventKind::System { subtype, text } => {
            format!("{subtype}: {}", text.clone().unwrap_or_default())
        }
        EventKind::Unknown { raw } => raw.to_string(),
        // `EventKind` is `#[non_exhaustive]`: a new variant should not break
        // the snapshot harness, only show up unlabelled until it is handled.
        _ => String::new(),
    };
    let single = text.replace('\n', " \u{21b5} ");
    single.chars().take(160).collect()
}

macro_rules! snapshot_fixture {
    ($test:ident, $fixture:literal) => {
        #[test]
        fn $test() {
            let session = parse($fixture);
            assert!(
                session.invariant_violations().is_empty(),
                "{}: {:?}",
                $fixture,
                session.invariant_violations()
            );
            insta::assert_json_snapshot!(stringify!($test), summarize(&session));
        }
    };
}

snapshot_fixture!(claude_basic, "claude/basic.jsonl");
snapshot_fixture!(claude_rewind, "claude/rewind.jsonl");
snapshot_fixture!(claude_compact_boundary, "claude/compact-boundary.jsonl");
snapshot_fixture!(
    claude_sidechain_and_malformed,
    "claude/sidechain-and-malformed.jsonl"
);
snapshot_fixture!(codex_basic, "codex/basic.jsonl");
snapshot_fixture!(codex_rollback, "codex/rollback.jsonl");
snapshot_fixture!(codex_ask_and_compaction, "codex/ask-and-compaction.jsonl");
snapshot_fixture!(pi_basic, "pi/basic.jsonl");
snapshot_fixture!(pi_branch, "pi/branch.jsonl");
snapshot_fixture!(pi_v1_linear, "pi/v1-linear.jsonl");

#[test]
fn every_fixture_is_detected_as_the_provider_that_wrote_it() {
    for (relative, expected) in [
        ("claude/basic.jsonl", AgentKind::ClaudeCode),
        ("claude/compact-boundary.jsonl", AgentKind::ClaudeCode),
        ("codex/basic.jsonl", AgentKind::Codex),
        ("codex/rollback.jsonl", AgentKind::Codex),
        ("pi/basic.jsonl", AgentKind::Pi),
        ("pi/v1-linear.jsonl", AgentKind::Pi),
    ] {
        assert_eq!(parse(relative).agent, expected, "{relative}");
    }
}

#[test]
fn a_rewind_keeps_only_the_live_path() {
    let session = parse("claude/rewind.jsonl");
    let live: Vec<String> = session
        .active_events()
        .map(|event| detail(&event.kind))
        .collect();
    let joined = live.join(" ~ ");
    assert!(joined.contains("keep it dependency free"), "{joined}");
    assert!(
        !joined.contains("p-retry"),
        "the abandoned branch survived: {joined}"
    );
    assert!(
        session
            .diagnostics
            .iter()
            .any(|d| matches!(d, Diagnostic::AbandonedBranch { .. })),
        "no abandoned-branch diagnostic: {:?}",
        session.diagnostics
    );
}

#[test]
fn a_compaction_boundary_does_not_truncate_the_history_before_it() {
    let session = parse("claude/compact-boundary.jsonl");
    let joined: String = session
        .active_events()
        .map(|event| detail(&event.kind))
        .collect::<Vec<_>>()
        .join(" ~ ");
    // The pre-boundary turn is reachable only through logicalParentUuid.
    assert!(joined.contains("start the auth migration"), "{joined}");
    assert!(joined.contains("Prior session summary"), "{joined}");
    assert_eq!(session.native_compactions.len(), 1);
}

#[test]
fn a_rollback_drops_the_undone_turn_and_its_work() {
    let session = parse("codex/rollback.jsonl");
    let joined: String = session
        .active_events()
        .map(|event| detail(&event.kind))
        .collect::<Vec<_>>()
        .join(" ~ ");
    assert!(joined.contains("ship the json flag"), "{joined}");
    assert!(joined.contains("add a --json flag"), "{joined}");
    assert!(
        !joined.contains("also add --xml"),
        "the rolled-back turn survived: {joined}"
    );
    assert!(
        !joined.contains("src/cli/xml.rs"),
        "rolled-back work survived: {joined}"
    );
}

#[test]
fn a_codex_question_and_its_answer_become_one_human_turn() {
    let session = parse("codex/ask-and-compaction.jsonl");
    let answers: Vec<&EventKind> = session
        .active_events()
        .map(|event| &event.kind)
        .filter(|kind| matches!(kind, EventKind::UserAnswer { .. }))
        .collect();
    assert_eq!(answers.len(), 1, "{answers:?}");
    match answers[0] {
        EventKind::UserAnswer { question, answer } => {
            assert!(question.contains("npm"), "{question}");
            assert_eq!(answer, "crates.io only for now");
        }
        other => panic!("unexpected kind: {other:?}"),
    }
    // Both human turns count: the request and the answer.
    assert_eq!(session.user_turns(), 2);
}

#[test]
fn codex_harness_context_is_never_counted_as_a_human_turn() {
    let session = parse("codex/basic.jsonl");
    assert_eq!(session.user_turns(), 1, "{:?}", session.kind_counts());
    let harness = session
        .active_events()
        .filter(|event| matches!(&event.kind, EventKind::UserMessage { is_meta: true, .. }))
        .count();
    assert_eq!(harness, 1);
    // Developer messages are harness instructions and are skipped entirely.
    let joined: String = session
        .active_events()
        .map(|event| detail(&event.kind))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(!joined.contains("You are Codex"), "{joined}");
}

#[test]
fn a_pi_branch_keeps_the_live_path_and_the_branch_summary() {
    let session = parse("pi/branch.jsonl");
    let joined: String = session
        .active_events()
        .map(|event| detail(&event.kind))
        .collect::<Vec<_>>()
        .join(" ~ ");
    assert!(joined.contains("call it budget"), "{joined}");
    assert!(
        !joined.contains("call it maxTokens"),
        "abandoned branch survived: {joined}"
    );
}

#[test]
fn a_malformed_line_is_a_diagnostic_not_a_failure() {
    let session = parse("claude/sidechain-and-malformed.jsonl");
    let bad = session
        .diagnostics
        .iter()
        .filter(|d| matches!(d, Diagnostic::BadLine { .. }))
        .count();
    assert_eq!(bad, 1, "{:?}", session.diagnostics);
    let unknown = session
        .diagnostics
        .iter()
        .filter(|d| matches!(d, Diagnostic::UnknownLineKind { .. }))
        .count();
    assert_eq!(unknown, 1, "{:?}", session.diagnostics);
    // The session is still usable.
    assert!(session.user_turns() >= 2);
}

#[test]
fn too_many_bad_lines_fails_with_exit_five() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("broken.jsonl");
    let mut body = String::from(
        "{\"type\":\"user\",\"uuid\":\"u1\",\"parentUuid\":null,\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n",
    );
    for _ in 0..20 {
        body.push_str("{not json\n");
    }
    std::fs::write(&path, body).expect("write");
    let error = adapters::parse_path(&path, 0.02).expect_err("should fail");
    assert_eq!(error.exit_code(), 5, "{error}");
}

#[test]
fn subagent_sidechains_get_their_own_stream_and_stay_out_of_the_main_one() {
    let session = parse("claude/sidechain-and-malformed.jsonl");
    let spawn = session
        .events
        .iter()
        .find(|event| matches!(event.kind, EventKind::SubagentSpawn { .. }))
        .expect("a Task call becomes a spawn");
    assert_eq!(spawn.stream, 0, "the spawn belongs to the main stream");
    let sidechain_streams: Vec<u32> = session
        .events
        .iter()
        .map(|event| event.stream)
        .filter(|stream| *stream > 0)
        .collect();
    assert!(
        !sidechain_streams.is_empty(),
        "no sidechain stream was assigned"
    );
    // Sidechain events are never on the main active branch.
    assert!(session.active_events().all(|event| event.stream == 0));
}

#[cfg(feature = "zstd")]
#[test]
fn a_zstd_compressed_rollout_parses_identically_to_the_plain_one() {
    let plain = std::fs::read(fixture("codex/basic.jsonl")).expect("read fixture");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir
        .path()
        .join("rollout-2026-09-01-6f1a2b3c-4d5e-6f70-8192-a3b4c5d6e7f8.jsonl.zst");
    std::fs::write(
        &path,
        zstd::stream::encode_all(&plain[..], 3).expect("compress"),
    )
    .expect("write");

    let compressed = adapters::parse_path(&path, 0.5).expect("parse compressed");
    let expected = parse("codex/basic.jsonl");
    assert_eq!(compressed.events.len(), expected.events.len());
    assert_eq!(compressed.active, expected.active);
    assert_eq!(compressed.id, expected.id);
    // The hash is over decompressed bytes, so it must match the plain file.
    assert_eq!(
        compressed.source_hash,
        source::combined_hash(&[source::read(&fixture("codex/basic.jsonl"))
            .expect("read")
            .hash])
    );
}
