//! CLI contract tests.
//!
//! Two properties matter to a calling agent and are pinned here: **stdout
//! carries the payload only**, and **exit codes mean what the spec says**.
//! Everything else about the CLI can change; these cannot, without a version
//! decision.

use assert_cmd::Command;
use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// A `sctxx` invocation that cannot see the developer's real agent stores.
///
/// `AGENTS.md` rule 4: tests never read real `~/.claude`, `~/.codex`, or
/// `~/.pi` stores. Pi's root is derived from `HOME`, so overriding the two
/// store environment variables is not enough — `HOME` itself must move.
fn sctxx() -> Command {
    let mut command = Command::cargo_bin("sctxx").expect("the binary builds");
    let empty = empty_store();
    command
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("SCTXX_API_KEY")
        .env_remove("SCTXX_LLM")
        .env("HOME", &empty)
        .env("USERPROFILE", &empty)
        .env("CLAUDE_CONFIG_DIR", &empty)
        .env("CODEX_HOME", &empty);
    command
}

/// A home directory that exists but holds no sessions.
fn empty_store() -> PathBuf {
    let path = std::env::temp_dir().join("sctxx-tests-empty-home");
    std::fs::create_dir_all(path.join("projects")).ok();
    std::fs::create_dir_all(path.join(".pi/agent/sessions")).ok();
    path
}

fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn help_and_version_succeed() {
    sctxx().arg("--help").assert().success();
    sctxx().arg("--version").assert().success();
}

#[test]
fn an_unknown_flag_is_a_usage_error() {
    sctxx().args(["list", "--nope"]).assert().code(2);
}

#[test]
fn extract_writes_only_the_artifact_to_stdout() {
    let output = sctxx()
        .args(["extract"])
        .arg(fixtures().join("claude/basic.jsonl"))
        .args(["--llm", "none", "--no-verify"])
        .output()
        .expect("run");

    assert!(output.status.success(), "{}", stderr_of(&output));
    let stdout = stdout_of(&output);
    // stdout must be the artifact and nothing else, so a caller can redirect it.
    assert!(
        stdout.starts_with("---\nschema: sctxx.handoff/v1"),
        "{stdout}"
    );
    assert!(stdout.contains("## L0 \u{b7} Brief"), "{stdout}");

    // Progress went to stderr.
    let stderr = stderr_of(&output);
    assert!(stderr.contains("[parse]"), "{stderr}");
    assert!(!stderr.contains("schema: sctxx.handoff/v1"), "{stderr}");
}

#[test]
fn quiet_silences_stderr_without_touching_stdout() {
    let output = sctxx()
        .args(["extract"])
        .arg(fixtures().join("claude/basic.jsonl"))
        .args(["--llm", "none", "--no-verify", "--quiet"])
        .output()
        .expect("run");
    assert!(output.status.success());
    assert!(stderr_of(&output).is_empty(), "{}", stderr_of(&output));
    assert!(stdout_of(&output).contains("sctxx.handoff/v1"));
}

#[test]
fn since_compact_starts_at_the_provider_boundary() {
    let output = sctxx()
        .args(["extract"])
        .arg(fixtures().join("codex/windowed-compaction.jsonl"))
        .args(["--llm", "none", "--no-verify", "--since-compact"])
        .output()
        .expect("run");
    assert!(output.status.success(), "{}", stderr_of(&output));

    // The run says where it started, because a caller cannot tell otherwise.
    let stderr = stderr_of(&output);
    assert!(stderr.contains("[since-compact]"), "{stderr}");
    assert!(
        stderr.contains("evt 1"),
        "the legacy reset at evt 1 should win over the window marker: {stderr}"
    );

    let stdout = stdout_of(&output);
    // The turn before the reset is outside the artifact...
    assert!(
        !stdout.contains("start the migration"),
        "pre-boundary history leaked in: {stdout}"
    );
    // ...and the turn after it is inside.
    assert!(stdout.contains("now do the second half"), "{stdout}");
}

#[test]
fn since_compact_is_a_notice_not_a_failure_when_nothing_compacted() {
    let output = sctxx()
        .args(["extract"])
        .arg(fixtures().join("codex/basic.jsonl"))
        .args(["--llm", "none", "--no-verify", "--since-compact"])
        .output()
        .expect("run");
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("has no provider compaction"),
        "{}",
        stderr_of(&output)
    );
    // The whole session is still extracted.
    assert!(stdout_of(&output).contains("sctxx.handoff/v1"));
}

#[test]
fn max_bad_lines_can_admit_a_session_the_default_rejects() {
    // This fixture has one truncated line in seven — 14.3%, well over the 2%
    // default — which is the shape a real session from a newer provider
    // version can take.
    let fixture = fixtures().join("claude/sidechain-and-malformed.jsonl");

    let rejected = sctxx()
        .args(["extract"])
        .arg(&fixture)
        .args(["--llm", "none", "--no-verify", "--quiet"])
        .output()
        .expect("run");
    assert_eq!(rejected.status.code(), Some(5), "{}", stderr_of(&rejected));
    // The message must name the way out, or a first run ends here.
    assert!(
        stderr_of(&rejected).contains("--max-bad-lines"),
        "{}",
        stderr_of(&rejected)
    );

    let admitted = sctxx()
        .args(["extract"])
        .arg(&fixture)
        .args([
            "--llm",
            "none",
            "--no-verify",
            "--quiet",
            "--max-bad-lines",
            "0.5",
        ])
        .output()
        .expect("run");
    assert!(admitted.status.success(), "{}", stderr_of(&admitted));
    assert!(stdout_of(&admitted).contains("sctxx.handoff/v1"));
}

#[test]
fn an_out_of_range_max_bad_lines_is_a_usage_error() {
    let output = sctxx()
        .args(["extract"])
        .arg(fixtures().join("claude/basic.jsonl"))
        .args(["--llm", "none", "--max-bad-lines", "5"])
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
    assert!(
        stderr_of(&output).contains("between 0 and 1"),
        "{}",
        stderr_of(&output)
    );
}

#[test]
fn writing_outside_a_repository_leaks_no_git_errors() {
    // The git-ignore check runs on every `--out`, and "not a repository" is a
    // normal answer to it, not something to print at the user. It used to:
    // `Command::status` inherits stderr, so git's own fatal message appeared
    // whenever the artifact was written outside a work tree.
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifacts");
    let output = sctxx()
        .args(["extract"])
        .arg(fixtures().join("claude/basic.jsonl"))
        .args(["--llm", "none", "--no-verify"])
        .arg("--out")
        .arg(&out)
        .output()
        .expect("run");
    assert!(output.status.success(), "{}", stderr_of(&output));
    let stderr = stderr_of(&output);
    assert!(!stderr.contains("fatal:"), "{stderr}");
    assert!(!stderr.contains("not a git repository"), "{stderr}");
}

#[test]
fn extract_warns_when_git_would_track_the_artifact() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path();
    if !git_available(repo) {
        return;
    }
    let out = repo.join(".sctxx");

    let warned = sctxx()
        .args(["extract"])
        .arg(fixtures().join("codex/basic.jsonl"))
        .args(["--llm", "none", "--no-verify"])
        .arg("--out")
        .arg(&out)
        .output()
        .expect("run");
    assert!(warned.status.success(), "{}", stderr_of(&warned));
    assert!(
        stderr_of(&warned).contains("not ignored by git"),
        "an artifact holding session content must not be silently committable: {}",
        stderr_of(&warned)
    );
    // The warning is a note, not the payload: stdout stays the artifact path.
    assert_eq!(
        stdout_of(&warned).trim(),
        out.join("handoff.md").display().to_string()
    );

    // Once the directory is ignored, the warning goes away.
    std::fs::write(repo.join(".git/info/exclude"), ".sctxx/\n").expect("write exclude");
    let quiet = sctxx()
        .args(["extract"])
        .arg(fixtures().join("codex/basic.jsonl"))
        .args(["--llm", "none", "--no-verify"])
        .arg("--out")
        .arg(&out)
        .output()
        .expect("run");
    assert!(quiet.status.success(), "{}", stderr_of(&quiet));
    assert!(
        !stderr_of(&quiet).contains("not ignored by git"),
        "{}",
        stderr_of(&quiet)
    );
}

/// Initialise a scratch repository, returning false when git is unavailable.
fn git_available(dir: &Path) -> bool {
    std::process::Command::new("git")
        .arg("init")
        .arg("-q")
        .arg(dir)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[test]
fn extract_to_a_directory_writes_five_files_and_prints_the_artifact_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join(".sctxx");
    let output = sctxx()
        .args(["extract"])
        .arg(fixtures().join("codex/basic.jsonl"))
        .args(["--llm", "none", "--no-verify", "--out"])
        .arg(&out)
        .output()
        .expect("run");

    assert!(output.status.success(), "{}", stderr_of(&output));
    for name in [
        "handoff.md",
        "handoff.json",
        "state.json",
        "ledgers.json",
        "report.json",
    ] {
        assert!(out.join(name).is_file(), "{name} missing");
    }
    // stdout is the path the receiving agent should read.
    assert_eq!(
        stdout_of(&output).trim(),
        out.join("handoff.md").to_string_lossy()
    );
}

#[test]
fn json_output_is_parseable_for_every_command_that_offers_it() {
    for args in [
        vec!["list", "--json"],
        vec!["find", "anything", "--json"],
        vec!["doctor", "--json"],
        vec!["schema", "handoff"],
        vec!["schema", "ops"],
        vec!["schema", "state"],
        vec!["schema", "ir"],
    ] {
        let output = sctxx().args(&args).output().expect("run");
        assert!(output.status.success(), "{args:?}: {}", stderr_of(&output));
        let stdout = stdout_of(&output);
        serde_json::from_str::<serde_json::Value>(&stdout)
            .unwrap_or_else(|error| panic!("{args:?} produced invalid JSON: {error}\n{stdout}"));
    }
}

#[test]
fn an_unknown_schema_name_is_a_usage_error() {
    sctxx().args(["schema", "nope"]).assert().code(2);
}

#[test]
fn a_missing_session_exits_four() {
    let output = sctxx()
        .args(["extract", "claude:nosuchsession"])
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(4), "{}", stderr_of(&output));
}

#[test]
fn an_ambiguous_reference_exits_three_with_candidates_on_stdout() {
    // Two sessions whose ids share a prefix.
    let store = tempfile::tempdir().expect("tempdir");
    let project = store.path().join("projects/-home-dev-x");
    std::fs::create_dir_all(&project).expect("mkdir");
    for suffix in ["aaa", "bbb"] {
        let body = serde_json::json!({
            "type": "user", "uuid": "u1", "parentUuid": null,
            "sessionId": format!("shared-prefix-{suffix}"),
            "cwd": "/home/dev/x", "timestamp": "2026-09-01T00:00:00Z",
            "message": {"role": "user", "content": "hello"}
        });
        std::fs::write(
            project.join(format!("shared-prefix-{suffix}.jsonl")),
            body.to_string(),
        )
        .expect("write");
    }

    let output = sctxx()
        .env("CLAUDE_CONFIG_DIR", store.path())
        .args(["show", "claude:shared-prefix", "--any-project"])
        .output()
        .expect("run");

    assert_eq!(output.status.code(), Some(3), "{}", stderr_of(&output));
    let candidates: serde_json::Value =
        serde_json::from_str(&stdout_of(&output)).expect("candidates are JSON on stdout");
    assert_eq!(candidates.as_array().map(Vec::len), Some(2), "{candidates}");
    assert!(stderr_of(&output).contains("matched 2 sessions"));
}

#[test]
fn a_file_that_is_not_a_session_exits_five() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("notes.jsonl");
    std::fs::write(&path, "just some text\n").expect("write");
    let output = sctxx().arg("show").arg(&path).output().expect("run");
    assert_eq!(output.status.code(), Some(5), "{}", stderr_of(&output));
}

#[test]
fn an_unusable_llm_selection_is_reported_as_a_usage_error() {
    let output = sctxx()
        .args(["extract"])
        .arg(fixtures().join("claude/basic.jsonl"))
        .args(["--llm", "api:nonsense"])
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
}

#[test]
fn show_renders_each_view_of_the_same_session() {
    let path = fixtures().join("pi/basic.jsonl");

    let raw = sctxx()
        .arg("show")
        .arg(&path)
        .args(["--view", "raw"])
        .output()
        .expect("run");
    assert!(raw.status.success(), "{}", stderr_of(&raw));
    // `raw` prints the provider's own lines, byte for byte.
    assert!(
        stdout_of(&raw).contains("\"parentId\""),
        "{}",
        stdout_of(&raw)
    );

    let masked = sctxx()
        .arg("show")
        .arg(&path)
        .args(["--view", "masked"])
        .output()
        .expect("run");
    assert!(
        stdout_of(&masked).contains("[user] Export durable items"),
        "{}",
        stdout_of(&masked)
    );
    // The masked view never shows raw JSON envelopes.
    assert!(!stdout_of(&masked).contains("\"parentId\""));

    let ir = sctxx()
        .arg("show")
        .arg(&path)
        .args(["--view", "ir"])
        .output()
        .expect("run");
    let events: serde_json::Value = serde_json::from_str(&stdout_of(&ir)).expect("ir is JSON");
    assert!(events.as_array().is_some_and(|events| !events.is_empty()));
}

#[test]
fn show_honors_an_event_range() {
    let output = sctxx()
        .arg("show")
        .arg(fixtures().join("claude/basic.jsonl"))
        .args(["--view", "masked", "--range", "0..1"])
        .output()
        .expect("run");
    let stdout = stdout_of(&output);
    assert!(stdout.contains("(evt 0)"), "{stdout}");
    assert!(!stdout.contains("(evt 9)"), "{stdout}");
}

#[test]
fn expand_resolves_a_pointer_from_an_artifact_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join(".sctxx");
    let session = fixtures().join("claude/basic.jsonl");

    sctxx()
        .args(["extract"])
        .arg(&session)
        .args(["--llm", "none", "--no-verify", "--out"])
        .arg(&out)
        .assert()
        .success();

    // The artifact records its own session, so expand needs only the path.
    let output = sctxx()
        .arg("expand")
        .arg(&session)
        .arg("9..10")
        .output()
        .expect("run");
    assert!(output.status.success(), "{}", stderr_of(&output));
    let stdout = stdout_of(&output);
    assert!(stdout.contains("=== evt 9"), "{stdout}");
    assert!(stdout.contains("pnpm vitest"), "{stdout}");
}

#[test]
fn expand_with_context_widens_the_window() {
    let narrow = sctxx()
        .arg("expand")
        .arg(fixtures().join("claude/basic.jsonl"))
        .arg("10..10")
        .output()
        .expect("run");
    let wide = sctxx()
        .arg("expand")
        .arg(fixtures().join("claude/basic.jsonl"))
        .args(["10..10", "--context", "3"])
        .output()
        .expect("run");
    assert!(stdout_of(&wide).len() > stdout_of(&narrow).len());
}

#[test]
fn a_reversed_range_is_a_usage_error() {
    sctxx()
        .arg("expand")
        .arg(fixtures().join("claude/basic.jsonl"))
        .arg("20..10")
        .assert()
        .code(2);
}

#[test]
fn redact_check_reports_secrets_without_writing_anything() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("leaky.jsonl");
    std::fs::write(
        &path,
        "{\"token\": \"ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789\"}\n",
    )
    .expect("write");

    let check = sctxx()
        .arg("redact")
        .arg(&path)
        .args(["--check", "--json"])
        .output()
        .expect("run");
    let report: serde_json::Value = serde_json::from_str(&stdout_of(&check)).expect("json");
    assert_eq!(report["clean"], false);
    assert!(report["redactions"].as_u64().unwrap_or(0) >= 1, "{report}");
    // --check never modifies the input.
    assert!(
        std::fs::read_to_string(&path)
            .expect("read")
            .contains("ghp_")
    );

    let redacted = sctxx().arg("redact").arg(&path).output().expect("run");
    assert!(stdout_of(&redacted).contains("[REDACTED_SECRET]"));
    assert!(!stdout_of(&redacted).contains("ghp_ABCDEF"));
}

#[test]
fn redact_reads_stdin() {
    let output = sctxx()
        .args(["redact", "-"])
        .write_stdin("api_key: 0123456789abcdef\n")
        .output()
        .expect("run");
    assert!(
        stdout_of(&output).contains("[REDACTED_SECRET]"),
        "{}",
        stdout_of(&output)
    );
}

#[test]
fn doctor_reports_stores_and_backends_without_printing_key_values() {
    let output = sctxx()
        .env(
            "ANTHROPIC_API_KEY",
            "sk-ant-secret-value-that-must-not-appear",
        )
        .args(["doctor"])
        .output()
        .expect("run");
    assert!(output.status.success());
    let stdout = stdout_of(&output);
    assert!(stdout.contains("Session stores"), "{stdout}");
    assert!(stdout.contains("ANTHROPIC_API_KEY"), "{stdout}");
    assert!(
        !stdout.contains("secret-value-that-must-not-appear"),
        "doctor leaked a key value"
    );
}

#[test]
fn skill_print_emits_the_skill_document() {
    let output = sctxx().args(["skill", "print"]).output().expect("run");
    let stdout = stdout_of(&output);
    assert!(stdout.starts_with("---"), "{stdout}");
    assert!(stdout.contains("name: sctxx"), "{stdout}");
}

#[test]
fn skill_install_is_idempotent_and_refuses_to_clobber_edits() {
    let home = tempfile::tempdir().expect("home");

    let first = sctxx()
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .args(["skill", "install", "--target", "codex", "--json"])
        .output()
        .expect("run");
    assert!(first.status.success(), "{}", stderr_of(&first));
    let installed = home.path().join(".agents/skills/sctxx/SKILL.md");
    assert!(installed.is_file());

    let second = sctxx()
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .args(["skill", "install", "--target", "codex"])
        .output()
        .expect("run");
    assert!(
        stdout_of(&second).contains("unchanged"),
        "{}",
        stdout_of(&second)
    );

    std::fs::write(&installed, "# edited by a human").expect("write");
    let third = sctxx()
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .args(["skill", "install", "--target", "codex"])
        .output()
        .expect("run");
    assert_eq!(third.status.code(), Some(1), "{}", stdout_of(&third));
    assert_eq!(
        std::fs::read_to_string(&installed).expect("read"),
        "# edited by a human"
    );
}

#[test]
fn dry_run_reports_the_plan_without_calling_a_model() {
    let output = sctxx()
        .args(["extract"])
        .arg(fixtures().join("claude/basic.jsonl"))
        .args(["--dry-run", "--json"])
        .output()
        .expect("run");
    assert!(output.status.success(), "{}", stderr_of(&output));
    let plan: serde_json::Value = serde_json::from_str(&stdout_of(&output)).expect("json");
    assert!(plan["masked_rows"].as_u64().unwrap_or(0) > 0, "{plan}");
    assert!(plan["events"].as_u64().unwrap_or(0) > 0, "{plan}");
    assert!(plan["llm"].is_string(), "{plan}");
}

#[test]
fn verify_re_checks_an_artifact_against_a_repository() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join(".sctxx");
    sctxx()
        .args(["extract"])
        .arg(fixtures().join("claude/basic.jsonl"))
        .args(["--llm", "none", "--no-verify", "--out"])
        .arg(&out)
        .assert()
        .success();

    let output = sctxx()
        .arg("verify")
        .arg(&out)
        .args(["--repo"])
        .arg(dir.path())
        .args(["--json"])
        .output()
        .expect("run");
    assert!(output.status.success(), "{}", stderr_of(&output));
    let report: serde_json::Value = serde_json::from_str(&stdout_of(&output)).expect("json");
    // The fixture's files do not exist in an empty directory: stale, not fatal.
    assert!(
        report["missing_files"]
            .as_array()
            .is_some_and(|files| !files.is_empty()),
        "{report}"
    );
}

#[test]
fn strict_verification_of_a_contradicted_artifact_exits_seven() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join(".sctxx");
    std::fs::create_dir_all(&out).expect("mkdir");
    // An artifact claiming a file was deleted, and a repository where it exists.
    std::fs::write(dir.path().join("still-here.rs"), "fn main() {}").expect("write");
    std::fs::write(
        out.join("handoff.json"),
        serde_json::json!({
            "schema": "sctxx.handoff/v1",
            "session": {"agent": "claude", "id": "x", "source_hash": "", "events": 1,
                        "active": 1, "user_turns": 1}
        })
        .to_string(),
    )
    .expect("write");
    std::fs::write(
        out.join("ledgers.json"),
        serde_json::json!({
            "files": [{"path": "still-here.rs", "ops": ["delete"], "created": false,
                       "deleted": true, "edits": 1, "reads": 0, "first_evt": 0,
                       "last_evt": 1, "inferred": false}]
        })
        .to_string(),
    )
    .expect("write");

    let output = sctxx()
        .arg("verify")
        .arg(&out)
        .args(["--repo"])
        .arg(dir.path())
        .arg("--strict")
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(7), "{}", stdout_of(&output));
}

#[test]
fn list_reports_an_empty_store_without_failing() {
    let output = sctxx().args(["list", "--json"]).output().expect("run");
    assert!(output.status.success());
    assert_eq!(stdout_of(&output).trim(), "[]");
    assert!(stderr_of(&output).contains("no sessions found"));
}

#[test]
fn list_and_find_read_a_store_root_override() {
    let store = fixtures().join("claude");
    let output = sctxx()
        .args(["list", "--json", "--any-project", "--claude-root"])
        .arg(&store)
        .output()
        .expect("run");
    let sessions: serde_json::Value = serde_json::from_str(&stdout_of(&output)).expect("json");
    assert!(
        sessions.as_array().is_some_and(|s| s.len() >= 4),
        "{sessions}"
    );

    let found = sctxx()
        .args([
            "find",
            "manifest loader",
            "--json",
            "--any-project",
            "--claude-root",
        ])
        .arg(&store)
        .output()
        .expect("run");
    let matched: serde_json::Value = serde_json::from_str(&stdout_of(&found)).expect("json");
    assert_eq!(matched.as_array().map(Vec::len), Some(1), "{matched}");
}

#[test]
fn tui_refuses_to_run_without_a_terminal() {
    // `assert_cmd` pipes stdout, which is exactly the case that must not hang:
    // an agent running `sctxx --tui` gets a sentence and exit 2, not a screen
    // waiting for a key press that will never come.
    let output = sctxx().arg("--tui").output().expect("run");
    assert_eq!(output.status.code(), Some(2), "{}", stderr_of(&output));
    // The refusal differs by build, and both are contracts: a build with the
    // viewport says why it cannot start, one without says the feature is absent.
    #[cfg(feature = "tui")]
    assert!(
        stderr_of(&output).contains("needs a terminal"),
        "the refusal must say why: {}",
        stderr_of(&output)
    );
    #[cfg(not(feature = "tui"))]
    assert!(
        stderr_of(&output).contains("without the `tui` feature"),
        "a minimal build must say the feature is missing: {}",
        stderr_of(&output)
    );
    assert!(
        stdout_of(&output).is_empty(),
        "--tui must not write a payload to stdout: {}",
        stdout_of(&output)
    );
}

#[test]
fn tui_is_a_top_level_flag_only() {
    // `sctxx --tui list` parses the flag and then a subcommand; it is a usage
    // error rather than a silently ignored flag.
    let with_subcommand = sctxx().args(["--tui", "list"]).output().expect("run");
    assert_eq!(
        with_subcommand.status.code(),
        Some(2),
        "{}",
        stderr_of(&with_subcommand)
    );

    // And it is not accepted after a subcommand at all.
    let after_subcommand = sctxx().args(["list", "--tui"]).output().expect("run");
    assert_eq!(
        after_subcommand.status.code(),
        Some(2),
        "{}",
        stderr_of(&after_subcommand)
    );
}

/// A named file destination is deliberate, so it gets that one file and no
/// git-ignore warning — and it is the branch `pipeline::write_destination`
/// newly owns for both the CLI and the TUI.
#[test]
fn extract_to_a_named_file_writes_only_that_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    for name in ["handoff.md", "handoff.json"] {
        let out = dir.path().join(name);
        let output = sctxx()
            .args(["extract"])
            .arg(fixtures().join("claude/basic.jsonl"))
            .args(["--llm", "none", "--no-verify", "--out"])
            .arg(&out)
            .output()
            .expect("run");

        assert!(output.status.success(), "{}", stderr_of(&output));
        assert!(out.is_file(), "{name} must exist");
        assert!(
            std::fs::metadata(&out).expect("metadata").len() > 0,
            "{name} must not be empty"
        );
        // The file itself is the payload, and there is nothing to warn about.
        assert_eq!(stdout_of(&output).trim(), out.display().to_string());
        assert!(
            !stderr_of(&output).contains("not ignored by git"),
            "a named file is not a surprising directory: {}",
            stderr_of(&output)
        );
        // Exactly one file, not a directory of them.
        for sibling in ["state.json", "ledgers.json", "report.json"] {
            assert!(
                !dir.path().join(sibling).exists(),
                "{sibling} must not be written beside a named file"
            );
        }
    }
}

/// The test binary lives in `target/`, which neither installer owns — so the
/// honest answer is that sctxx cannot update itself, and it says which commands
/// would work instead of guessing at a package manager.
#[test]
fn update_refuses_a_build_that_was_not_installed() {
    let output = sctxx().args(["update", "--check"]).output().expect("run");
    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout={} stderr={}",
        stdout_of(&output),
        stderr_of(&output)
    );
    let stderr = stderr_of(&output);
    assert!(stderr.contains("cargo install sctxx"), "{stderr}");
    assert!(stderr.contains("npm install -g sctxx@latest"), "{stderr}");
    assert!(
        stdout_of(&output).trim().is_empty(),
        "a refusal is not payload: {}",
        stdout_of(&output)
    );
}

#[test]
fn update_check_reports_the_cargo_command_and_runs_nothing() {
    // A real cargo-shaped layout: the binary in `$CARGO_HOME/bin`, which is
    // exactly how `Method::of` recognises a `cargo install`. `--check` must
    // print the command and stop, which is what makes this test safe — it never
    // spawns a package manager.
    let cargo_home = tempfile::tempdir().expect("tempdir");
    let bin = cargo_home.path().join("bin");
    std::fs::create_dir_all(&bin).expect("create bin");
    let name = if cfg!(windows) { "sctxx.exe" } else { "sctxx" };
    let installed = bin.join(name);
    std::fs::copy(assert_cmd::cargo::cargo_bin("sctxx"), &installed).expect("copy the binary");

    let output = std::process::Command::new(&installed)
        .args(["update", "--check"])
        .env("CARGO_HOME", cargo_home.path())
        .output()
        .expect("run");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        stdout_of(&output),
        stderr_of(&output)
    );
    assert_eq!(stdout_of(&output).trim(), "cargo install sctxx --force");
    assert!(
        stderr_of(&output).contains("cargo"),
        "the reason must be on stderr: {}",
        stderr_of(&output)
    );
}

/// A fake agent CLI that reports the version the seeding channel was verified
/// on, so the plan path can be tested on a machine that has no agents at all.
#[cfg(unix)]
fn fake_agent_dir(version: &str) -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tempdir");
    let program = dir.path().join("claude");
    std::fs::write(
        &program,
        format!("#!/bin/sh\necho '{version} (Claude Code)'\n"),
    )
    .expect("write");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    dir
}

#[test]
fn handoff_answers_who_could_continue_a_session() {
    // The question worth being able to ask without having decided yet. This is
    // what the TUI's picker shows, for a caller that is not a person.
    let output = sctxx()
        .args(["handoff"])
        .arg(fixtures().join("claude/basic.jsonl"))
        .arg("--json")
        .output()
        .expect("run");
    assert!(output.status.success(), "{}", stderr_of(&output));

    let value: serde_json::Value =
        serde_json::from_str(&stdout_of(&output)).expect("stdout is one JSON object");
    assert!(
        value["session"]
            .as_str()
            .unwrap_or_default()
            .starts_with("claude:"),
        "{value}"
    );
    assert!(value["agents"].is_array(), "{value}");
    assert!(
        value["session_path"].is_string(),
        "a caller needs the file too: {value}"
    );
}

#[test]
fn handoff_refuses_an_agent_it_cannot_launch() {
    let output = sctxx()
        .args(["handoff"])
        .arg(fixtures().join("claude/basic.jsonl"))
        .args(["--to", "definitely-not-an-agent"])
        .output()
        .expect("run");
    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout={} stderr={}",
        stdout_of(&output),
        stderr_of(&output)
    );
    assert!(
        stdout_of(&output).trim().is_empty(),
        "a refusal is not payload: {}",
        stdout_of(&output)
    );
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("definitely-not-an-agent") && stderr.contains("claude"),
        "the refusal must name what is available: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn handoff_plans_a_real_command_without_running_it() {
    let agents = fake_agent_dir("2.1.268");
    let out = tempfile::tempdir().expect("tempdir");
    let path = format!(
        "{}:{}",
        agents.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let output = sctxx()
        .env("PATH", path)
        .args(["handoff"])
        .arg(fixtures().join("claude/basic.jsonl"))
        .args(["--to", "claude", "--json", "--out"])
        .arg(out.path())
        .output()
        .expect("run");
    assert!(output.status.success(), "{}", stderr_of(&output));

    let value: serde_json::Value = serde_json::from_str(&stdout_of(&output)).expect("json");
    assert_eq!(value["agent"], "claude");
    assert_eq!(value["ran"], false, "planning never launches");
    assert_eq!(value["fallback"], false, "2.1.268 is the verified version");
    assert!(
        value["program"]
            .as_str()
            .unwrap_or_default()
            .contains("claude"),
        "{value}"
    );
    let argv = value["argv"].as_array().expect("argv");
    assert_eq!(argv[0], "--append-system-prompt-file");
    assert!(
        argv[1].as_str().unwrap_or_default().ends_with("handoff.md"),
        "the artifact travels as a path: {argv:?}"
    );
    assert!(
        value["cwd"].as_str().is_some(),
        "a caller needs to know where to start it: {value}"
    );

    // And the artifact is really there, because the plan is not a promise.
    let artifact = value["artifact"].as_str().expect("artifact");
    assert!(Path::new(artifact).is_file(), "{artifact}");

    // Asking again reuses it rather than redoing the extraction.
    let again = sctxx()
        .env(
            "PATH",
            format!(
                "{}:{}",
                agents.path().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .args(["handoff"])
        .arg(fixtures().join("claude/basic.jsonl"))
        .args(["--to", "claude", "--json", "--out"])
        .arg(out.path())
        .output()
        .expect("run");
    let value: serde_json::Value = serde_json::from_str(&stdout_of(&again)).expect("json");
    assert_eq!(
        value["reused"], true,
        "an artifact for this session is reused"
    );
}

#[test]
fn extract_is_deterministic_unless_a_model_is_asked_for() {
    // The default that used to spend 813,000 tokens on a large session without
    // saying so.
    let output = sctxx()
        .args(["extract"])
        .arg(fixtures().join("claude/basic.jsonl"))
        .args(["--dry-run", "--json"])
        .output()
        .expect("run");
    assert!(output.status.success(), "{}", stderr_of(&output));
    let value: serde_json::Value = serde_json::from_str(&stdout_of(&output)).expect("json");
    assert_eq!(
        value["llm"], "none",
        "the default must not call a model: {value}"
    );
    assert_eq!(value["planned_fold_calls"], 0, "{value}");
    assert_eq!(value["planned_premap_calls"], 0, "{value}");
    assert_eq!(
        value["estimated_prompt_tokens"], 0,
        "nothing is sent, so nothing is estimated: {value}"
    );
}
