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

/// How long one completion may take before the child is killed.
const TIMEOUT: Duration = Duration::from_secs(600);

/// How to invoke one agent CLI as a completion engine.
///
/// Flags differ per CLI and change between versions; each template documents
/// the version it was verified against so a breakage is diagnosable.
#[derive(Debug)]
struct Template {
    name: &'static str,
    program: &'static str,
    args: &'static [&'static str],
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
        args: &["-p", "--output-format", "json", "--allowed-tools", ""],
        extract: Extract::JsonKey("result"),
    },
    Template {
        name: "codex",
        program: "codex",
        args: &[
            "exec",
            "--skip-git-repo-check",
            "--sandbox",
            "read-only",
            "-",
        ],
        extract: Extract::Stdout,
    },
    Template {
        name: "pi",
        program: "pi",
        args: &["-p"],
        extract: Extract::Stdout,
    },
];

/// An agent CLI used as a completion engine.
#[derive(Debug)]
pub struct CliBackend {
    template: &'static Template,
    program: PathBuf,
}

impl CliBackend {
    /// Look up `name` in the template table and on `PATH`.
    pub fn new(name: &str) -> Result<Self> {
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
        Ok(Self { template, program })
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

        let output = wait_with_timeout(child, TIMEOUT).map_err(|message| Error::LlmFailed {
            backend: self.name(),
            message,
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::LlmFailed {
                backend: self.name(),
                message: format!(
                    "exited with {}: {}",
                    output.status.code().unwrap_or(-1),
                    crate::vendor::codex::truncate::truncate_middle_bytes(stderr.trim(), 500)
                ),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
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
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".to_string())
            .split(';')
            .map(|ext| ext.to_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };
    for directory in std::env::split_paths(&path) {
        for extension in &extensions {
            let candidate = directory.join(format!("{program}{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
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
