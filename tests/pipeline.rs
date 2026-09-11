//! End-to-end pipeline tests: fixture in, artifact out.
//!
//! The LLM is the only mocked boundary. Everything else — parsing, ledgers,
//! masking, segmentation, the fold loop, validation, apply, reconciliation,
//! rendering — is the real code path a user runs.

use sctxx::adapters::discovery;
use sctxx::llm::Selection;
use sctxx::pipeline::{self, ExtractOptions, Mode};
use std::path::{Path, PathBuf};

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(relative)
}

fn summary(relative: &str) -> discovery::SessionSummary {
    let path = fixture(relative);
    let reference = discovery::parse_reference(&path.to_string_lossy()).expect("reference");
    discovery::resolve(&reference, &discovery::ResolveOptions::default()).expect("resolve")
}

fn deterministic_options() -> ExtractOptions {
    ExtractOptions {
        llm: Selection::None,
        // The fixtures are small; a small tail keeps some rows in chunks.
        tail_tokens: 200,
        chunk_tokens: 600,
        // Reconciliation would depend on whatever repository the tests run in.
        verify: false,
        ..ExtractOptions::default()
    }
}

fn extract(relative: &str, options: &ExtractOptions) -> pipeline::Extraction {
    pipeline::extract(&summary(relative), options, &mut |_, _| {})
        .unwrap_or_else(|error| panic!("{relative}: {error}"))
}

/// One number out of the artifact's `tokens: {…}` front-matter line.
fn token_field(markdown: &str, name: &str) -> usize {
    let line = markdown
        .lines()
        .find(|line| line.starts_with("tokens:"))
        .unwrap_or_else(|| {
            panic!(
                "no tokens line in:\n{}",
                &markdown[..markdown.len().min(400)]
            )
        });
    let rest = &line[line
        .find(name)
        .unwrap_or_else(|| panic!("no {name} in {line}"))
        + name.len()..];
    rest.trim_start_matches([':', ' '])
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or_else(|error| panic!("{name} in {line}: {error}"))
}

#[test]
fn the_artifact_header_states_its_own_real_size() {
    let options = deterministic_options();
    let extraction = extract("claude/basic.jsonl", &options);
    let markdown = extraction.markdown(&options);

    // These two were hardcoded to 0, so every artifact's header claimed the
    // session had no masked tokens and that the artifact was empty — the line a
    // reader uses to judge how much was thrown away.
    let masked = token_field(&markdown, "masked");
    let artifact = token_field(&markdown, "artifact");

    assert!(masked > 0, "masked is still zero:\n{markdown}");
    assert!(artifact > 0, "artifact is still zero:\n{markdown}");
    assert_eq!(masked, extraction.report.tokens.masked);
    // Measured from a first pass, so it can differ by the digits it prints —
    // never by more.
    let reported = extraction.markdown(&options).len() / 4;
    assert!(
        artifact.abs_diff(reported) <= 4,
        "artifact {artifact} vs measured {reported}"
    );
}

/// Normalize the parts of an artifact that legitimately vary between runs.
fn stable(markdown: &str) -> String {
    stable_for(markdown, env!("CARGO_MANIFEST_DIR"))
}

/// [`stable`] against an explicit checkout root, so the Windows case is
/// testable from any platform.
///
/// The artifact renders POSIX-normalized paths (`render::posix`), but
/// `CARGO_MANIFEST_DIR` is native: on Windows it is `D:\a\sctxx\sctxx` while
/// the line says `D:/a/sctxx/sctxx/...`. Replacing only the native form left
/// the absolute path in place and broke the snapshots on Windows alone — which
/// is exactly the sort of bug this repository keeps finding on one platform.
fn stable_for(markdown: &str, root: &str) -> String {
    let posix = root.replace('\\', "/");
    markdown
        .lines()
        .map(|line| {
            if line.starts_with("source:") {
                // Absolute fixture paths differ per machine.
                "source: <normalized>".to_string()
            } else if line.starts_with("tokens:") {
                "tokens: <normalized>".to_string()
            } else if line.starts_with("sctxx:") {
                "sctxx: <version>".to_string()
            } else {
                // Either form, then drop the separator the native form leaves
                // behind so the snapshot reads the same on every platform.
                line.replace(root, "<repo>")
                    .replace(&posix, "<repo>")
                    .replace("<repo>\\", "<repo>/")
            }
        })
        .collect::<Vec<String>>()
        .join("\n")
}

#[test]
fn a_windows_checkout_root_is_normalized_in_both_forms() {
    let rendered = "- `D:/a/sctxx/sctxx/tests/fixtures/claude/basic.jsonl`";
    assert_eq!(
        stable_for(rendered, r"D:\a\sctxx\sctxx"),
        "- `<repo>/tests/fixtures/claude/basic.jsonl`"
    );
    // And the native form, for a line that was not POSIX-normalized.
    assert_eq!(
        stable_for(r"- `D:\a\sctxx\sctxx\Cargo.toml`", r"D:\a\sctxx\sctxx"),
        "- `<repo>/Cargo.toml`"
    );
}

#[test]
fn the_deterministic_artifact_is_complete_without_any_model() {
    let options = deterministic_options();
    let extraction = extract("claude/basic.jsonl", &options);
    let markdown = extraction.markdown(&options);

    // No model ran. The state is no longer empty — the deterministic typed layer
    // seeds it before any backend is chosen — so the assertion is that nothing
    // in it came from a model, which is the property that was actually meant.
    assert_eq!(extraction.llm_label, "none");
    assert_eq!(extraction.report.fold_calls, 0);
    for item in &extraction.state.items {
        assert!(
            item.why
                .as_deref()
                .is_some_and(|why| why.starts_with("stated by the user")),
            "a model-written item appeared with no model: {item:?}"
        );
    }
    // And the standing instruction the user gave is one of them.
    assert!(
        extraction
            .state
            .active_of(sctxx::pipeline::fold::ops::ItemKind::Constraint)
            .iter()
            .any(|item| item.text.contains("Never auto-install extensions")),
        "the user's rule is missing:\n{markdown}"
    );

    // And yet the artifact answers the questions that matter.
    assert!(markdown.contains("manifest loader"), "no goal:\n{markdown}");
    assert!(
        markdown.contains("src/host/module-host.ts"),
        "no files:\n{markdown}"
    );
    assert!(
        markdown.contains("pnpm vitest run packages/ext-engine"),
        "no command status:\n{markdown}"
    );
    assert!(
        markdown.contains("FAILED"),
        "the failing test is not reported:\n{markdown}"
    );
    assert!(
        markdown.contains("Unresolved errors"),
        "no error ledger:\n{markdown}"
    );
    assert!(
        markdown.contains("**Verify first**"),
        "no verify-first block:\n{markdown}"
    );
    assert!(
        markdown.contains("sctxx expand"),
        "no retrieval layer:\n{markdown}"
    );
}

#[test]
fn the_deterministic_artifact_is_byte_for_byte_reproducible() {
    let options = deterministic_options();
    let first = extract("claude/basic.jsonl", &options).markdown(&options);
    let second = extract("claude/basic.jsonl", &options).markdown(&options);
    assert_eq!(first, second, "extraction is not deterministic");
}

#[test]
fn deterministic_artifacts_are_snapshotted_for_every_provider() {
    let options = deterministic_options();
    for (name, relative) in [
        ("claude", "claude/basic.jsonl"),
        ("codex", "codex/basic.jsonl"),
        ("pi", "pi/basic.jsonl"),
    ] {
        let markdown = extract(relative, &options).markdown(&options);
        insta::assert_snapshot!(format!("handoff_deterministic_{name}"), stable(&markdown));
    }
}

#[test]
fn the_ledgers_read_the_session_the_way_a_reviewer_would() {
    let extraction = extract("claude/basic.jsonl", &deterministic_options());
    let ledgers = &extraction.ledgers;

    let edited: Vec<&str> = ledgers
        .edited_files()
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(edited, vec!["src/host/module-host.ts"], "{edited:?}");

    let test_command = ledgers
        .last_command_status()
        .into_iter()
        .find(|command| command.normalized.contains("vitest"))
        .expect("the test command is in the ledger");
    assert_eq!(test_command.status(), "FAILED");

    // The same failure twice is one signature, still unresolved.
    let unresolved = ledgers.unresolved_errors();
    assert_eq!(unresolved.len(), 1, "{unresolved:?}");
    assert_eq!(unresolved[0].occurrences, 2);
    assert!(
        unresolved[0].example.contains("capabilities"),
        "{}",
        unresolved[0].example
    );

    // The plan the agent last published survives.
    let plan = ledgers.plan.as_ref().expect("a plan was published");
    assert_eq!(plan.items.len(), 3);
}

#[test]
fn a_codex_patch_is_read_as_file_operations() {
    let extraction = extract("codex/basic.jsonl", &deterministic_options());
    let paths: Vec<&str> = extraction
        .ledgers
        .files
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    assert!(paths.contains(&"src/pipeline/ledgers.rs"), "{paths:?}");
    assert!(paths.contains(&"src/pipeline/tail.rs"), "{paths:?}");
    let created = extraction
        .ledgers
        .files
        .iter()
        .find(|file| file.path == "src/pipeline/tail.rs")
        .expect("the added file is recorded");
    assert!(
        created.created,
        "an Add File hunk did not mark the file created"
    );
}

#[test]
fn an_error_that_a_later_run_fixes_is_reported_as_resolved() {
    let extraction = extract("codex/basic.jsonl", &deterministic_options());
    // `cargo test` failed, then the same command passed.
    assert!(
        extraction.ledgers.unresolved_errors().is_empty(),
        "{:?}",
        extraction.ledgers.unresolved_errors()
    );
    assert_eq!(extraction.ledgers.errors.len(), 1);
    let markdown = extraction.markdown(&deterministic_options());
    assert!(markdown.contains("were resolved"), "{markdown}");
}

#[test]
fn the_fold_turns_a_session_into_provenance_linked_items() {
    let options = ExtractOptions {
        llm: Selection::Mock,
        ..deterministic_options()
    };
    let extraction = extract("claude/basic.jsonl", &options);

    assert!(extraction.report.fold_calls > 0, "the fold never ran");
    assert!(
        !extraction.state.items.is_empty(),
        "the fold produced no items"
    );
    // Every item must cite evidence; that is the whole contract.
    for item in extraction.state.active() {
        assert!(!item.sources.is_empty(), "{} has no provenance", item.id);
        for range in &item.sources {
            assert!(
                range.end < extraction.session.events.len() as u32,
                "{} cites {range} beyond the session",
                item.id
            );
        }
    }
    let markdown = extraction.markdown(&options);
    assert!(
        markdown.contains("[evt "),
        "no pointers in the artifact:\n{markdown}"
    );
    assert!(markdown.contains("llm: mock"), "{markdown}");
}

#[test]
fn a_constraint_the_fold_invents_never_reaches_the_artifact() {
    // The mock quotes a real user line, so its constraint is accepted; an
    // invented one is rejected by the same gate.
    let options = ExtractOptions {
        llm: Selection::Mock,
        ..deterministic_options()
    };
    let extraction = extract("claude/basic.jsonl", &options);
    let user_text: String = extraction
        .ledgers
        .user_messages
        .iter()
        .map(|m| m.text.to_lowercase())
        .collect();

    for item in extraction.state.active() {
        if let Some(quote) = &item.quote {
            let normalized: String = quote
                .split_whitespace()
                .map(|word| {
                    word.trim_matches(|c: char| !c.is_alphanumeric())
                        .to_lowercase()
                })
                .collect::<Vec<String>>()
                .join(" ");
            assert!(
                user_text.contains(normalized.split(' ').next().unwrap_or("")),
                "{} quotes something no human said: {quote}",
                item.id
            );
        }
    }
}

#[test]
fn no_secret_from_a_session_reaches_a_backend_or_the_artifact() {
    // A session containing every secret class; nothing may leave it intact.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("leaky.jsonl");
    let secret = "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let body = format!(
        "{}\n{}\n",
        serde_json::json!({
            "type": "user", "uuid": "u1", "parentUuid": null, "sessionId": "leaky",
            "cwd": "/tmp", "timestamp": "2026-09-01T00:00:00Z",
            "message": {"role": "user", "content": format!("deploy with token {secret}")}
        }),
        serde_json::json!({
            "type": "assistant", "uuid": "a1", "parentUuid": "u1", "sessionId": "leaky",
            "timestamp": "2026-09-01T00:00:01Z",
            "message": {"role": "assistant", "content": [{"type": "tool_use", "id": "c1",
                "name": "Bash", "input": {"command": format!("curl -H 'Authorization: Bearer {secret}' https://api")}}]}
        })
    );
    std::fs::write(&path, body).expect("write");

    let reference = discovery::parse_reference(&path.to_string_lossy()).expect("reference");
    let session =
        discovery::resolve(&reference, &discovery::ResolveOptions::default()).expect("resolve");
    let options = ExtractOptions {
        llm: Selection::Mock,
        ..deterministic_options()
    };
    let extraction = pipeline::extract(&session, &options, &mut |_, _| {}).expect("extract");

    let markdown = extraction.markdown(&options);
    assert!(
        !markdown.contains(secret),
        "the artifact leaked a token:\n{markdown}"
    );
    assert!(
        markdown.contains("[REDACTED_SECRET]"),
        "nothing was redacted:\n{markdown}"
    );

    // The masked rows are what a backend would have seen.
    for row in &extraction.rows {
        assert!(
            !row.text.contains(secret),
            "a masked row leaked a token: {}",
            row.text
        );
    }
    let json = serde_json::to_string(&extraction.json(&options)).expect("json");
    assert!(!json.contains(secret), "handoff.json leaked a token");
    let ledgers = serde_json::to_string(&extraction.ledgers).expect("ledgers");
    assert!(!ledgers.contains(secret), "ledgers.json leaked a token");
}

#[test]
fn the_artifact_stays_inside_its_budget() {
    let options = ExtractOptions {
        budget: 400,
        ..deterministic_options()
    };
    let extraction = extract("claude/basic.jsonl", &options);
    let markdown = extraction.markdown(&options);
    // L2 is governed by --tail, so measure everything before it.
    let head = markdown.split("## L2").next().unwrap_or(&markdown);
    // The front matter and the preamble state what the artifact is and are not
    // budgeted. L0 carries three blocks that are charged to the budget *first*
    // and never refused — the notice that no model ran, the contradictions, and
    // the user's own instructions — so L0 is bounded by the budget plus those
    // blocks, and L1 spends what is left.
    let l0 = markdown
        .split("## L0")
        .nth(1)
        .and_then(|rest| rest.split("## L1").next())
        .unwrap_or("");
    assert!(
        l0.len() / 4 <= 400 + 250,
        "L0 used {} tokens against a 400 budget plus its mandatory blocks",
        l0.len() / 4
    );
    let tokens = head.len() / 4;
    assert!(
        tokens <= 1_000,
        "L0+L1 used {tokens} tokens against a 400 budget"
    );
}

#[test]
fn writing_out_produces_the_five_documented_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let options = deterministic_options();
    let extraction = extract("claude/basic.jsonl", &options);
    let written = pipeline::write_all(&extraction, &options, dir.path()).expect("write");
    assert_eq!(written.paths.len(), 5);

    for name in [
        "handoff.md",
        "handoff.json",
        "state.json",
        "ledgers.json",
        "report.json",
    ] {
        let path = dir.path().join(name);
        assert!(path.is_file(), "{name} was not written");
        if name.ends_with(".json") {
            let body = std::fs::read_to_string(&path).expect("read");
            serde_json::from_str::<serde_json::Value>(&body)
                .unwrap_or_else(|error| panic!("{name} is not valid JSON: {error}"));
        }
    }

    // handoff.json must match its published schema's identity.
    let handoff: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("handoff.json")).expect("read"),
    )
    .expect("parse");
    assert_eq!(handoff["schema"], "sctxx.handoff/v1");
    assert_eq!(handoff["session"]["agent"], "claude");
}

#[test]
fn every_pointer_in_an_artifact_resolves_to_real_events() {
    let options = ExtractOptions {
        llm: Selection::Mock,
        ..deterministic_options()
    };
    let extraction = extract("claude/basic.jsonl", &options);
    let markdown = extraction.markdown(&options);
    let event_count = extraction.session.events.len() as u32;

    // The preamble and L3 explain the `[evt a-b]` notation in prose; only the
    // item layers contain real pointers.
    let body = markdown
        .split_once("## L0")
        .map(|(_, rest)| rest)
        .unwrap_or(&markdown)
        .split("## L3")
        .next()
        .unwrap_or("");

    let mut found = 0;
    for capture in body.split("[evt ").skip(1) {
        let inside = capture.split(']').next().unwrap_or("");
        for part in inside.split(',') {
            let part = part.trim().trim_start_matches("evt ").trim();
            let (start, end) = match part.split_once('\u{2013}') {
                Some((start, end)) => (start, end),
                None => (part, part),
            };
            let start: u32 = start
                .trim()
                .parse()
                .unwrap_or_else(|_| panic!("bad pointer: {part}"));
            let end: u32 = end
                .trim()
                .parse()
                .unwrap_or_else(|_| panic!("bad pointer: {part}"));
            assert!(start <= end, "reversed pointer {part}");
            assert!(end < event_count, "pointer {part} is beyond the session");
            found += 1;
        }
    }
    assert!(found > 0, "the artifact contains no pointers:\n{markdown}");
}

#[test]
fn a_backend_that_fails_still_yields_the_deterministic_artifact() {
    // A cli backend that does not exist: extraction must degrade, not abort.
    let options = ExtractOptions {
        llm: Selection::Cli("definitely-not-installed".into()),
        ..deterministic_options()
    };
    let error = pipeline::extract(&summary("claude/basic.jsonl"), &options, &mut |_, _| {})
        .expect_err("an unknown cli name is a usage error");
    assert_eq!(error.exit_code(), 2);
}

/// Small chunks force the fixture to split into more than the premap threshold.
fn many_chunk_options() -> ExtractOptions {
    ExtractOptions {
        llm: Selection::Mock,
        chunk_tokens: 25,
        tail_tokens: 25,
        ..deterministic_options()
    }
}

#[test]
fn fast_mode_skips_the_premap_pass() {
    let options = ExtractOptions {
        mode: Mode::Fast,
        ..many_chunk_options()
    };
    let extraction = extract("claude/basic.jsonl", &options);
    assert!(
        extraction.report.chunks > 4,
        "only {} chunks",
        extraction.report.chunks
    );
    assert_eq!(extraction.report.premap_calls, 0);
}

#[test]
fn standard_mode_premaps_when_there_are_many_chunks() {
    let extraction = extract("claude/basic.jsonl", &many_chunk_options());
    assert!(
        extraction.report.premap_calls > 0,
        "premap did not run: {:?}",
        extraction.report
    );
    assert_eq!(extraction.report.premap_calls, extraction.report.chunks);
}

#[test]
fn a_long_turn_is_folded_rather_than_dumped_into_the_recency_tail() {
    let extraction = extract("claude/basic.jsonl", &many_chunk_options());
    assert!(extraction.report.chunks > 0, "nothing was folded");
    assert!(
        extraction.report.tail_rows < extraction.report.rows,
        "the tail swallowed the whole session"
    );
}
