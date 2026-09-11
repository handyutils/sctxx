//! `cli:<name>` backends (spec §9.3).
//!
//! Runs an installed agent CLI non-interactively as a plain completion engine.
//! This is the backend most users will have: it reuses the subscription login
//! they already pay for and needs no API key.
//!
//! Two safety properties matter and are enforced here:
//!
//! 1. The subprocess runs in an **empty temporary directory**, so an agent
//!    acting as an LLM cannot read or modify the user's repository.
//! 2. The child is killed if it outlives its call (timeout or a dropped
//!    handle), so no orphaned agent process survives sctxx.

use super::{Backend, Capabilities, Request, Response};
use crate::error::{Error, Result};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Which agent CLIs `auto` will try, in order (spec §9.6).
pub const PREFERENCE_ORDER: &[&str] = &["claude", "codex", "pi"];

/// How long one completion may take before the child is killed, unless the
/// caller says otherwise.
///
/// A fold call over a long session is a large prompt asking for a large
/// structured answer, and 600s is not enough for it: on a real 103,757-event
/// session both a 53k-token and a 25k-token chunk call were killed at exactly
/// this mark while the much smaller tail pass finished in about thirty seconds.
/// The work is discarded when that happens, so the caller can raise it.
pub const DEFAULT_TIMEOUT_SECS: u64 = 600;

/// How to invoke one agent CLI as a completion engine.
///
/// Flags differ per CLI and change between versions; each template records the
/// version it was verified against so a breakage is diagnosable.
#[derive(Debug)]
struct Template {
    name: &'static str,
    program: &'static str,
    args: &'static [&'static str],
    /// The agent CLI version this argv was verified on.
    verified_against: &'static str,
    /// Where the response text lives in stdout.
    extract: Extract,
}

#[derive(Debug)]
enum Extract {
    /// The whole of stdout.
    Stdout,
    /// A JSON object on stdout, at this top-level key.
    JsonKey(&'static str),
}

const TEMPLATES: &[Template] = &[
    Template {
        name: "claude",
        program: "claude",
        // `-p` is one-shot print mode; the tool allowlist is emptied so the
        // subprocess cannot touch the filesystem even inside its temp cwd.
        //
        // `--no-session-persistence` is not optional. Claude Code records a
        // session per working directory, so without it every completion writes
        // a session into `~/.claude/projects` whose transcript is sctxx's own
        // prompt — and `sctxx list` then reports sctxx's own scratch calls as
        // real sessions. The scratch cwd does not prevent this; it only names
        // the pollution.
        args: &[
            "-p",
            "--output-format",
            "json",
            "--allowed-tools",
            "",
            "--no-session-persistence",
        ],
        verified_against: "2.1.268",
        extract: Extract::JsonKey("result"),
    },
    Template {
        name: "codex",
        program: "codex",
        // `--ephemeral` is `codex exec`'s "run without persisting session files
        // to disk", for the same reason as `claude` above; `-` reads the prompt
        // from stdin.
        args: &[
            "exec",
            "--skip-git-repo-check",
            "--sandbox",
            "read-only",
            "--ephemeral",
            "-",
        ],
        verified_against: "0.153.4",
        extract: Extract::Stdout,
    },
    Template {
        name: "pi",
        program: "pi",
        // `--no-session` is Pi's "don't save session (ephemeral)".
        args: &["-p", "--no-session"],
        verified_against: "0.85.1",
        extract: Extract::Stdout,
    },
];

/// An agent CLI used as a completion engine.
#[derive(Debug)]
pub struct CliBackend {
    template: &'static Template,
    program: PathBuf,
    timeout: Duration,
}

impl CliBackend {
    /// Look up `name` in the template table and on `PATH`, with the default
    /// timeout.
    pub fn new(name: &str) -> Result<Self> {
        Self::with_timeout(name, Duration::from_secs(DEFAULT_TIMEOUT_SECS))
    }

    /// As [`CliBackend::new`], with an explicit completion timeout.
    pub fn with_timeout(name: &str, timeout: Duration) -> Result<Self> {
        let template = TEMPLATES
            .iter()
            .find(|template| template.name == name)
            .ok_or_else(|| {
                Error::Usage(format!(
                    "unknown cli backend `{name}` (expected one of {})",
                    PREFERENCE_ORDER.join(", ")
                ))
            })?;
        let program = find_executable(template.program).ok_or_else(|| {
            Error::LlmUnavailable(format!("`{}` is not on PATH", template.program))
        })?;
        Ok(Self {
            template,
            program,
            timeout,
        })
    }
}

impl Backend for CliBackend {
    fn name(&self) -> String {
        format!("cli:{}", self.template.name)
    }

    fn capabilities(&self) -> Capabilities {
        // No agent CLI exposes structured output as a completion engine, so
        // the schema is asked for in the prompt and validated after parsing.
        Capabilities {
            json_schema_native: false,
            max_context: None,
        }
    }

    fn complete(&self, request: &Request) -> Result<Response> {
        // An empty cwd: the agent cannot see the user's repository from here.
        let workdir = tempdir()?;
        let prompt = format!("{}\n\n{}", request.system, request.user);

        let mut child = Command::new(&self.program)
            .args(self.template.args)
            .current_dir(workdir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| Error::LlmFailed {
                backend: self.name(),
                message: format!("spawn failed: {error}"),
            })?;

        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(prompt.as_bytes())
                .map_err(|error| Error::LlmFailed {
                    backend: self.name(),
                    message: format!("writing the prompt failed: {error}"),
                })?;
        }
        // Closing stdin is what tells a one-shot CLI to start work.
        drop(child.stdin.take());

        let timeout = self.timeout;
        let output = wait_with_timeout(child, timeout).map_err(|message| Error::LlmFailed {
            backend: self.name(),
            message,
        })?;

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        if !output.status.success() {
            return Err(Error::LlmFailed {
                backend: self.name(),
                message: format!(
                    "exited with {} (cli:{} is verified against {}): {}",
                    output.status.code().unwrap_or(-1),
                    self.template.name,
                    self.template.verified_against,
                    // The reason is not always on stderr. Claude Code exits 1
                    // with an *empty* stderr and puts the reason in its JSON on
                    // stdout, so reading stderr alone reported a failure with no
                    // message at all — which then looked like an empty fold.
                    failure_reason(&stdout)
                        .or_else(|| failure_reason(&stderr))
                        .unwrap_or_else(|| "no output on either stream".to_string())
                ),
            });
        }

        // A CLI can report an API error in JSON and still exit 0. Returning that
        // text as if it were the model's answer would feed the fold an error
        // message to parse — and a fold that parses nothing produces nothing.
        if let Some(reason) = reported_error(&stdout) {
            return Err(Error::LlmFailed {
                backend: self.name(),
                message: format!("the CLI reported an error: {reason}"),
            });
        }

        let text = match self.template.extract {
            Extract::Stdout => stdout,
            Extract::JsonKey(key) => serde_json::from_str::<serde_json::Value>(&stdout)
                .ok()
                .and_then(|value| value.get(key).and_then(|v| v.as_str()).map(str::to_string))
                // A CLI that changed its output shape should still be usable.
                .unwrap_or(stdout),
        };
        Ok(Response {
            text,
            input_tokens: None,
            output_tokens: None,
        })
    }
}

/// The reason a CLI failed, from whichever stream carries it.
///
/// A JSON error object with a `result`, `error`, or `message` field is read for
/// that field; anything else is used as-is. Empty text is not a reason.
fn failure_reason(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        for key in ["result", "error", "message", "detail"] {
            if let Some(found) = value.get(key).and_then(|v| v.as_str())
                && !found.trim().is_empty()
            {
                // The status code is the difference between "you are rate
                // limited" and "you are logged out".
                let status = value
                    .get("api_error_status")
                    .map(|s| format!(" (status {s})"))
                    .unwrap_or_default();
                return Some(format!("{}{status}", found.trim()));
            }
        }
    }
    Some(crate::vendor::codex::truncate::truncate_middle_bytes(
        trimmed, 500,
    ))
}

/// An error a CLI reported inside an otherwise successful exit.
fn reported_error(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text.trim()).ok()?;
    if value.get("is_error").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }
    failure_reason(text)
}

/// Wait for a child, killing it if it exceeds `timeout`.
fn wait_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
) -> std::result::Result<std::process::Output, String> {
    let deadline = std::time::Instant::now() + timeout;
    // `wait_with_output` has no timeout, so poll and drain at the end. The
    // child's stdout pipe is large enough for one completion; a pathological
    // producer hits the timeout and is killed.
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("timed out after {}s", timeout.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => {
                let _ = child.kill();
                return Err(format!("waiting for the child failed: {error}"));
            }
        }
    }
    child
        .wait_with_output()
        .map_err(|error| format!("reading child output failed: {error}"))
}

/// A temporary directory that removes itself when dropped.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn tempdir() -> Result<TempDir> {
    // A counter plus the pid is enough: the directory is private to this call.
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("sctxx-llm-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(&path).map_err(|source| Error::io(&path, source))?;
    Ok(TempDir { path })
}

/// Find an executable on `PATH`, honoring Windows extensions.
pub fn find_executable(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    // The same question the handoff agent detector asks, so the two cannot
    // disagree about whether a binary exists.
    crate::agents::find_program(&dirs, program)
}

/// Agent CLIs detected on this machine, for `sctxx doctor`.
pub fn detected() -> Vec<(&'static str, PathBuf)> {
    PREFERENCE_ORDER
        .iter()
        .filter_map(|name| find_executable(name).map(|path| (*name, path)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_suppresses_session_persistence() {
        // This is a guard, not a formality. Claude Code records a session per
        // working directory, so a template without its switch writes one into
        // the user's own store for every completion — and `sctxx list` then
        // shows sctxx's private scratch prompts as sessions. Dropping the flag
        // must fail here rather than reappear as junk in the user's history.
        let expected = [
            ("claude", "--no-session-persistence"),
            ("codex", "--ephemeral"),
            ("pi", "--no-session"),
        ];
        assert_eq!(
            TEMPLATES.len(),
            expected.len(),
            "a template was added or removed without a persistence flag"
        );
        for (name, flag) in expected {
            let template = TEMPLATES
                .iter()
                .find(|template| template.name == name)
                .unwrap_or_else(|| panic!("no `{name}` template"));
            assert!(
                template.args.contains(&flag),
                "cli:{name} must pass `{flag}` so the call is not recorded as a session"
            );
        }
    }

    #[test]
    fn every_template_records_the_version_it_was_verified_against() {
        // These flags change between releases without notice, so the version is
        // what makes a breakage diagnosable from the error alone.
        for template in TEMPLATES {
            assert!(
                template
                    .verified_against
                    .chars()
                    .any(|c| c.is_ascii_digit()),
                "cli:{} must record the version it was verified against",
                template.name
            );
        }
    }

    #[test]
    fn a_failure_reason_is_read_from_whichever_stream_has_it() {
        // The shape Claude Code actually produces on a rate limit: exit 1, an
        // empty stderr, and the reason inside its JSON on stdout.
        let claude =
            r#"{"is_error":true,"api_error_status":429,"result":"You've hit your session limit"}"#;
        assert_eq!(
            failure_reason(claude).as_deref(),
            Some("You've hit your session limit (status 429)")
        );
        assert_eq!(
            failure_reason("plain text error").as_deref(),
            Some("plain text error")
        );
        // Silence is not a reason, and must not be reported as one.
        assert_eq!(failure_reason(""), None);
        assert_eq!(failure_reason("   \n "), None);
    }

    #[test]
    fn an_error_reported_with_a_successful_exit_is_still_an_error() {
        assert!(reported_error(r#"{"is_error":true,"result":"nope"}"#).is_some());
        // A normal answer is not an error, whatever it says.
        assert!(reported_error(r#"{"result":"all good"}"#).is_none());
        assert!(reported_error("plain model text").is_none());
    }

    #[test]
    fn an_unknown_cli_name_is_a_usage_error() {
        let error = CliBackend::new("nope").expect_err("should reject");
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn a_missing_executable_reports_the_backend_as_unavailable() {
        // `pi` may legitimately exist on a developer machine; only assert the
        // error shape when it does not.
        if find_executable("pi").is_none() {
            let error = CliBackend::new("pi").expect_err("should be unavailable");
            assert_eq!(error.exit_code(), 6);
        }
    }

    #[test]
    fn path_lookup_finds_a_real_program() {
        let program = if cfg!(windows) { "cmd" } else { "sh" };
        assert!(
            find_executable(program).is_some(),
            "{program} not found on PATH"
        );
        assert!(find_executable("definitely-not-a-real-program-xyz").is_none());
    }

    #[test]
    fn a_temp_dir_is_removed_when_it_drops() {
        let path = {
            let dir = tempdir().expect("tempdir");
            assert!(dir.path().is_dir());
            dir.path().to_path_buf()
        };
        assert!(!path.exists(), "temp dir survived its owner");
    }

    #[test]
    fn a_child_that_never_exits_is_killed_at_the_timeout() {
        if cfg!(windows) {
            return;
        }
        let child = Command::new("sh")
            .args(["-c", "sleep 30"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        let error = wait_with_timeout(child, Duration::from_millis(200)).expect_err("timeout");
        assert!(error.contains("timed out"), "{error}");
    }
}
