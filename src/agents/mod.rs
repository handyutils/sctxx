//! The coding agents installed on this machine.
//!
//! This is new code with nothing to reuse. `agentman` checks only whether a
//! store *directory* exists under `$HOME`, and the maintainer's launcher reads a
//! catalogue of ~30 agents without versions. A handoff needs more than either:
//! whether the binary is on `PATH`, which version it answers with, and whether
//! that version is one sctxx has actually verified a seeding channel for
//! ([ADR 0004](../../docs/adr/0004-handoff-launch-and-seeding.md)).
//!
//! Nothing is inferred from a directory. A store that exists with no binary is
//! not an install, and an unverified version selects the documented fallback
//! rather than a template that may no longer exist.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long a binary gets to answer `--version` before it is killed.
///
/// Detection runs while a developer waits, and a CLI that hangs on `--version`
/// must not hang the tool that asked.
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// One agent sctxx knows how to look for.
struct Candidate {
    id: &'static str,
    label: &'static str,
    /// The binary name, looked up on `PATH`.
    program: &'static str,
    /// The environment variable that relocates this agent's home, if it has one.
    env_var: &'static str,
    /// The session store under that variable's directory.
    env_store: &'static str,
    /// The session store under `$HOME` when the variable is unset.
    home_store: &'static str,
    /// The version ADR 0004 verified a seeding channel against.
    verified_against: &'static str,
}

/// The agents sctxx can hand off to, and how to find each one.
///
/// The store paths are the same ones `adapters::discovery` reads, and the
/// environment overrides are the ones those CLIs document (`CLAUDE_CONFIG_DIR`
/// replaces `~/.claude`; `CODEX_HOME` replaces `~/.codex`).
const CANDIDATES: &[Candidate] = &[
    Candidate {
        id: "claude",
        label: "Claude Code",
        program: "claude",
        env_var: "CLAUDE_CONFIG_DIR",
        env_store: "projects",
        home_store: ".claude/projects",
        verified_against: "2.1.268",
    },
    Candidate {
        id: "codex",
        label: "Codex CLI",
        program: "codex",
        env_var: "CODEX_HOME",
        env_store: "sessions",
        home_store: ".codex/sessions",
        verified_against: "0.153.4",
    },
    Candidate {
        id: "pi",
        label: "Pi",
        program: "pi",
        env_var: "",
        env_store: "",
        home_store: ".pi/agent/sessions",
        verified_against: "0.85.1",
    },
];

/// What sctxx found for one agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub id: &'static str,
    pub label: &'static str,
    /// The binary, when it is on `PATH`.
    pub program: Option<PathBuf>,
    /// The version the binary reported, when it answered in time.
    pub version: Option<String>,
    /// Where this agent keeps sessions, whether or not it is there.
    pub store: PathBuf,
    pub store_exists: bool,
    /// The version ADR 0004 verified a seeding channel against.
    pub verified_against: &'static str,
}

impl Agent {
    /// True when the binary is on `PATH`.
    ///
    /// This is the question that matters, and it is not the same as "the store
    /// directory exists" — which is all `agentman` asks, and which is wrong in
    /// both directions.
    pub fn installed(&self) -> bool {
        self.program.is_some()
    }

    /// True when this is the exact version a seeding channel was verified on.
    pub fn version_verified(&self) -> bool {
        self.version.as_deref() == Some(self.verified_against)
    }

    /// One line for `doctor`.
    pub fn status(&self) -> String {
        match (&self.program, &self.version) {
            (None, _) => "not installed".to_string(),
            (Some(_), None) => format!(
                "installed; version unknown, so the cwd handoff fallback applies \
                 (verified on {})",
                self.verified_against
            ),
            (Some(_), Some(version)) if self.version_verified() => {
                format!("installed at {version}; seeding verified on this version")
            }
            (Some(_), Some(version)) => format!(
                "installed at {version}; seeding verified on {}, so the cwd fallback applies",
                self.verified_against
            ),
        }
    }
}

/// Where to look. A struct so a test can describe a machine instead of being run
/// on one.
#[derive(Debug, Clone, Default)]
pub struct Machine {
    pub path_dirs: Vec<PathBuf>,
    pub home: Option<PathBuf>,
    /// The environment variables that relocate an agent's home.
    pub env: BTreeMap<String, PathBuf>,
}

impl Machine {
    /// This machine.
    pub fn this_one() -> Self {
        let path_dirs = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default();
        let env = CANDIDATES
            .iter()
            .filter(|candidate| !candidate.env_var.is_empty())
            .filter_map(|candidate| {
                std::env::var_os(candidate.env_var)
                    .map(|value| (candidate.env_var.to_string(), PathBuf::from(value)))
            })
            .collect();
        Self {
            path_dirs,
            home: home_dir(),
            env,
        }
    }

    /// Find every agent sctxx knows about, installed or not.
    ///
    /// All of them are returned, with the reason they are or are not usable,
    /// because a handoff screen that silently omits an agent is worse than one
    /// that says the binary is missing.
    pub fn detect(&self) -> Vec<Agent> {
        CANDIDATES
            .iter()
            .map(|candidate| {
                let program = find_program(&self.path_dirs, candidate.program);
                let version = program.as_deref().and_then(version_of);
                let store = self.store_for(candidate);
                Agent {
                    id: candidate.id,
                    label: candidate.label,
                    program,
                    version,
                    store_exists: store.is_dir(),
                    store,
                    verified_against: candidate.verified_against,
                }
            })
            .collect()
    }

    fn store_for(&self, candidate: &Candidate) -> PathBuf {
        if !candidate.env_var.is_empty()
            && let Some(directory) = self.env.get(candidate.env_var)
        {
            return directory.join(candidate.env_store);
        }
        match &self.home {
            Some(home) => home.join(candidate.home_store),
            None => PathBuf::from(candidate.home_store),
        }
    }
}

/// Find an executable in these directories, honoring Windows extensions.
///
/// Shared with the `cli:` LLM backends, which ask the same question of the same
/// three binaries.
pub fn find_program(dirs: &[PathBuf], program: &str) -> Option<PathBuf> {
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".to_string())
            .split(';')
            .map(|extension| extension.to_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };
    for directory in dirs {
        for extension in &extensions {
            let candidate = directory.join(format!("{program}{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Read the version a binary reports, without waiting forever for it.
fn version_of(program: &Path) -> Option<String> {
    let mut child = Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let deadline = Instant::now() + VERSION_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => {
                let _ = child.kill();
                return None;
            }
        }
    }

    let output = child.wait_with_output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    // A few CLIs print their version to stderr; take whichever said something.
    let text = if stdout.trim().is_empty() {
        String::from_utf8_lossy(&output.stderr).into_owned()
    } else {
        stdout.into_owned()
    };
    parse_version(&text)
}

/// The version in a `--version` string.
///
/// The three CLIs spell it differently — `2.1.268 (Claude Code)`,
/// `codex-cli 0.153.4`, `0.85.1` — so the rule is the shape of a version, not
/// the shape of a line: the last whitespace-separated token that begins with a
/// digit and contains a dot.
pub fn parse_version(text: &str) -> Option<String> {
    text.lines()
        .next()?
        .split_whitespace()
        .rfind(|token| token.starts_with(|c: char| c.is_ascii_digit()) && token.contains('.'))
        .map(|token| {
            token
                .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.')
                .to_string()
        })
        .filter(|token| !token.is_empty())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine(home: &Path) -> Machine {
        Machine {
            path_dirs: Vec::new(),
            home: Some(home.to_path_buf()),
            env: BTreeMap::new(),
        }
    }

    #[test]
    fn the_three_installers_spell_their_version_differently_and_all_parse() {
        // The real strings, captured from the versions ADR 0004 was verified on.
        assert_eq!(
            parse_version("2.1.268 (Claude Code)").as_deref(),
            Some("2.1.268")
        );
        assert_eq!(
            parse_version("codex-cli 0.153.4").as_deref(),
            Some("0.153.4")
        );
        assert_eq!(parse_version("0.85.1").as_deref(), Some("0.85.1"));
        assert_eq!(parse_version("0.85.1\n").as_deref(), Some("0.85.1"));
        // Nothing that looks like a version is not an error, it is an absence.
        assert_eq!(parse_version("command not found"), None);
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("claude version unknown"), None);
    }

    #[test]
    fn a_store_directory_with_no_binary_is_not_an_install() {
        // The distinction agentman does not make, and the reason this is new
        // code rather than borrowed: a leftover store directory must not
        // advertise an agent that cannot be launched.
        let home = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(home.path().join(".claude/projects")).expect("store");

        let agents = machine(home.path()).detect();
        let claude = agents
            .iter()
            .find(|agent| agent.id == "claude")
            .expect("claude");
        assert!(claude.store_exists, "the store is there");
        assert!(!claude.installed(), "but the binary is not on PATH");
        assert!(
            claude.status().contains("not installed"),
            "{}",
            claude.status()
        );
    }

    #[test]
    fn every_agent_is_reported_even_when_none_is_installed() {
        // A handoff screen that silently omits an agent is worse than one that
        // says the binary is missing.
        let home = tempfile::tempdir().expect("tempdir");
        let agents = machine(home.path()).detect();
        assert_eq!(agents.len(), CANDIDATES.len());
        assert!(agents.iter().all(|agent| !agent.installed()));
        assert!(agents.iter().all(|agent| agent.version.is_none()));
    }

    #[test]
    fn an_environment_override_moves_the_store() {
        let home = tempfile::tempdir().expect("tempdir");
        let elsewhere = tempfile::tempdir().expect("tempdir");
        let mut machine = machine(home.path());
        machine
            .env
            .insert("CODEX_HOME".to_string(), elsewhere.path().to_path_buf());

        let agents = machine.detect();
        let codex = agents
            .iter()
            .find(|agent| agent.id == "codex")
            .expect("codex");
        assert_eq!(codex.store, elsewhere.path().join("sessions"));
        // And the agents without the variable set keep the home-relative path.
        let pi = agents.iter().find(|agent| agent.id == "pi").expect("pi");
        assert_eq!(pi.store, home.path().join(".pi/agent/sessions"));
    }

    #[test]
    fn claude_config_dir_replaces_the_whole_claude_directory() {
        // Not `~/.claude` plus a suffix: the variable relocates the agent's
        // home, so the store is `projects` directly under it.
        let home = tempfile::tempdir().expect("tempdir");
        let configured = tempfile::tempdir().expect("tempdir");
        let mut machine = machine(home.path());
        machine.env.insert(
            "CLAUDE_CONFIG_DIR".to_string(),
            configured.path().to_path_buf(),
        );

        let agents = machine.detect();
        let claude = agents
            .iter()
            .find(|agent| agent.id == "claude")
            .expect("claude");
        assert_eq!(claude.store, configured.path().join("projects"));
    }

    #[test]
    fn an_unverified_version_selects_the_fallback_and_says_so() {
        let home = tempfile::tempdir().expect("tempdir");
        let agent = Agent {
            id: "claude",
            label: "Claude Code",
            program: Some(PathBuf::from("/usr/local/bin/claude")),
            version: Some("9.9.9".to_string()),
            store: home.path().to_path_buf(),
            store_exists: false,
            verified_against: "2.1.268",
        };
        assert!(agent.installed());
        assert!(!agent.version_verified());
        let status = agent.status();
        assert!(status.contains("9.9.9"), "{status}");
        assert!(status.contains("fallback"), "{status}");
        assert!(status.contains("2.1.268"), "{status}");
    }

    #[test]
    fn a_verified_version_is_reported_as_such() {
        let home = tempfile::tempdir().expect("tempdir");
        let agent = Agent {
            id: "pi",
            label: "Pi",
            program: Some(PathBuf::from("/usr/local/bin/pi")),
            version: Some("0.85.1".to_string()),
            store: home.path().to_path_buf(),
            store_exists: true,
            verified_against: "0.85.1",
        };
        assert!(agent.version_verified());
        assert!(
            agent.status().contains("seeding verified"),
            "{}",
            agent.status()
        );
    }

    #[test]
    fn a_binary_with_no_readable_version_is_installed_but_unverified() {
        let home = tempfile::tempdir().expect("tempdir");
        let agent = Agent {
            id: "codex",
            label: "Codex CLI",
            program: Some(PathBuf::from("/usr/local/bin/codex")),
            version: None,
            store: home.path().to_path_buf(),
            store_exists: false,
            verified_against: "0.153.4",
        };
        assert!(agent.installed(), "the binary is there");
        assert!(!agent.version_verified());
        assert!(
            agent.status().contains("version unknown"),
            "{}",
            agent.status()
        );
    }

    #[test]
    fn path_lookup_finds_a_real_program() {
        let dir = tempfile::tempdir().expect("tempdir");
        let name = if cfg!(windows) {
            "claude.exe"
        } else {
            "claude"
        };
        let program = dir.path().join(name);
        std::fs::write(&program, b"#!/bin/sh\n").expect("write");
        assert_eq!(
            find_program(&[dir.path().to_path_buf()], "claude"),
            Some(program)
        );
        assert_eq!(find_program(&[dir.path().to_path_buf()], "codex"), None);
    }

    /// End to end on a real process: a script that answers `--version` the way
    /// Claude Code does must be detected, and one that answers nonsense must be
    /// installed but unverified.
    #[cfg(unix)]
    #[test]
    fn a_real_binary_is_detected_and_its_version_read() {
        use std::os::unix::fs::PermissionsExt;

        let home = tempfile::tempdir().expect("tempdir");
        let bin = tempfile::tempdir().expect("tempdir");
        let program = bin.path().join("claude");
        std::fs::write(&program, "#!/bin/sh\necho '2.1.268 (Claude Code)'\n").expect("write");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        let mut machine = machine(home.path());
        machine.path_dirs = vec![bin.path().to_path_buf()];
        let agents = machine.detect();
        let claude = agents
            .iter()
            .find(|agent| agent.id == "claude")
            .expect("claude");

        assert_eq!(claude.version.as_deref(), Some("2.1.268"));
        assert!(claude.version_verified(), "{}", claude.status());
    }

    /// A binary that never answers must not hang detection.
    #[cfg(unix)]
    #[test]
    fn a_binary_that_hangs_is_abandoned_rather_than_waited_for() {
        use std::os::unix::fs::PermissionsExt;

        let home = tempfile::tempdir().expect("tempdir");
        let bin = tempfile::tempdir().expect("tempdir");
        let program = bin.path().join("claude");
        // Longer than the timeout, so the kill is what ends it.
        std::fs::write(&program, "#!/bin/sh\nsleep 30\n").expect("write");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        let mut machine = machine(home.path());
        machine.path_dirs = vec![bin.path().to_path_buf()];

        let started = Instant::now();
        let agents = machine.detect();
        let claude = agents
            .iter()
            .find(|agent| agent.id == "claude")
            .expect("claude");
        assert!(claude.installed(), "the binary was found");
        assert_eq!(claude.version, None, "but it never answered");
        assert!(
            started.elapsed() < VERSION_TIMEOUT + Duration::from_secs(5),
            "detection must not wait for a hung binary"
        );
    }
}
