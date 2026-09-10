//! Agent Skill packaging (spec §13).
//!
//! The CLI is the engine; the skill is a thin router that teaches an agent
//! *when* and *how* to call it. Installing writes the same `SKILL.md` into
//! whichever agents' skill directories exist, and refuses to clobber a file a
//! human has edited.

use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

/// The skill document, embedded so a published binary can install itself.
pub const SKILL_MD: &str = include_str!("../skill/SKILL.md");
/// Generated CLI reference shipped alongside the skill.
pub const CLI_REFERENCE: &str = include_str!("../skill/references/cli.md");
/// Artifact-format reference shipped alongside the skill.
pub const ARTIFACT_REFERENCE: &str = include_str!("../skill/references/artifact.md");

/// Marker file recording which sctxx version installed a skill.
const VERSION_MARKER: &str = ".sctxx-skill-version";

/// An agent that can load skills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Target {
    ClaudeCode,
    Codex,
    Pi,
}

impl Target {
    pub fn slug(self) -> &'static str {
        match self {
            Target::ClaudeCode => "claude",
            Target::Codex => "codex",
            Target::Pi => "pi",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Target::ClaudeCode => "Claude Code",
            Target::Codex => "Codex CLI",
            Target::Pi => "Pi",
        }
    }

    pub const ALL: [Target; 3] = [Target::ClaudeCode, Target::Codex, Target::Pi];

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "claude" | "claude-code" => Ok(Target::ClaudeCode),
            "codex" => Ok(Target::Codex),
            "pi" => Ok(Target::Pi),
            other => Err(Error::Usage(format!(
                "unknown skill target `{other}` (expected claude, codex, or pi)"
            ))),
        }
    }

    /// Where this agent looks for skills.
    ///
    /// OpenCode reads Claude Code's directories, so it needs no target of its
    /// own.
    pub fn skill_dir(self, scope: Scope, home: &Path, project: &Path) -> Option<PathBuf> {
        let name = "sctxx";
        match (self, scope) {
            (Target::ClaudeCode, Scope::User) => Some(home.join(".claude/skills").join(name)),
            (Target::ClaudeCode, Scope::Project) => Some(project.join(".claude/skills").join(name)),
            (Target::Codex, Scope::User) => Some(home.join(".agents/skills").join(name)),
            (Target::Codex, Scope::Project) => Some(project.join(".agents/skills").join(name)),
            (Target::Pi, Scope::User) => Some(home.join(".pi/agent/skills").join(name)),
            // Pi has no documented project scope.
            (Target::Pi, Scope::Project) => None,
        }
    }
}

/// Whether to install for the user or for one project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Scope {
    User,
    Project,
}

impl Scope {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "user" => Ok(Scope::User),
            "project" => Ok(Scope::Project),
            other => Err(Error::Usage(format!(
                "unknown scope `{other}` (expected user or project)"
            ))),
        }
    }
}

/// What happened for one target.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Outcome {
    Installed {
        path: String,
    },
    Updated {
        path: String,
        from: String,
    },
    Unchanged {
        path: String,
    },
    /// The file was edited by a human; `--force` overrides.
    Modified {
        path: String,
    },
    Skipped {
        reason: String,
    },
    Removed {
        path: String,
    },
}

/// Install the skill for `targets`.
pub fn install(
    targets: &[Target],
    scope: Scope,
    home: &Path,
    project: &Path,
    force: bool,
) -> Vec<(Target, Outcome)> {
    targets
        .iter()
        .map(|target| {
            let outcome = match target.skill_dir(scope, home, project) {
                Some(dir) => install_one(&dir, force),
                None => Outcome::Skipped {
                    reason: format!("{} has no {:?} scope", target.label(), scope),
                },
            };
            (*target, outcome)
        })
        .collect()
}

fn install_one(dir: &Path, force: bool) -> Outcome {
    let skill_path = dir.join("SKILL.md");
    let marker_path = dir.join(VERSION_MARKER);
    let installed_version = std::fs::read_to_string(&marker_path)
        .ok()
        .map(|v| v.trim().to_string());

    if skill_path.exists() && !force {
        let existing = std::fs::read_to_string(&skill_path).unwrap_or_default();
        if existing == SKILL_MD {
            return Outcome::Unchanged {
                path: display(&skill_path),
            };
        }
        // A file we did not write, or one a human changed after we wrote it.
        let ours = installed_version.is_some();
        if !ours {
            return Outcome::Modified {
                path: display(&skill_path),
            };
        }
        // We wrote it and it differs from what we would write now: either a
        // human edited it or sctxx was upgraded. Only the upgrade is safe to
        // apply silently, and we cannot tell the two apart, so ask.
        if installed_version.as_deref() != Some(crate::VERSION) {
            match write_files(dir) {
                Ok(()) => {
                    return Outcome::Updated {
                        path: display(&skill_path),
                        from: installed_version.unwrap_or_default(),
                    };
                }
                Err(error) => {
                    return Outcome::Skipped {
                        reason: error.to_string(),
                    };
                }
            }
        }
        return Outcome::Modified {
            path: display(&skill_path),
        };
    }

    match write_files(dir) {
        Ok(()) => Outcome::Installed {
            path: display(&skill_path),
        },
        Err(error) => Outcome::Skipped {
            reason: error.to_string(),
        },
    }
}

fn write_files(dir: &Path) -> Result<()> {
    let references = dir.join("references");
    std::fs::create_dir_all(&references).map_err(|source| Error::io(&references, source))?;
    for (name, body) in [
        ("SKILL.md", SKILL_MD),
        ("references/cli.md", CLI_REFERENCE),
        ("references/artifact.md", ARTIFACT_REFERENCE),
        (VERSION_MARKER, crate::VERSION),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, body).map_err(|source| Error::io(&path, source))?;
    }
    Ok(())
}

/// Remove an installed skill.
pub fn uninstall(
    targets: &[Target],
    scope: Scope,
    home: &Path,
    project: &Path,
) -> Vec<(Target, Outcome)> {
    targets
        .iter()
        .map(|target| {
            let outcome = match target.skill_dir(scope, home, project) {
                Some(dir) if dir.exists() => match std::fs::remove_dir_all(&dir) {
                    Ok(()) => Outcome::Removed {
                        path: display(&dir),
                    },
                    Err(error) => Outcome::Skipped {
                        reason: error.to_string(),
                    },
                },
                Some(dir) => Outcome::Skipped {
                    reason: format!("{} is not installed", display(&dir)),
                },
                None => Outcome::Skipped {
                    reason: "no such scope".to_string(),
                },
            };
            (*target, outcome)
        })
        .collect()
}

fn display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_writes_the_skill_and_its_references() {
        let home = tempfile::tempdir().expect("home");
        let project = tempfile::tempdir().expect("project");
        let results = install(
            &Target::ALL,
            Scope::User,
            home.path(),
            project.path(),
            false,
        );
        assert_eq!(results.len(), 3);
        for (target, outcome) in &results {
            assert!(
                matches!(outcome, Outcome::Installed { .. }),
                "{target:?}: {outcome:?}"
            );
        }
        let skill = home.path().join(".claude/skills/sctxx/SKILL.md");
        assert!(skill.is_file());
        assert!(
            home.path()
                .join(".claude/skills/sctxx/references/cli.md")
                .is_file()
        );
        assert_eq!(
            std::fs::read_to_string(
                home.path()
                    .join(".claude/skills/sctxx/.sctxx-skill-version")
            )
            .expect("marker"),
            crate::VERSION
        );
    }

    #[test]
    fn reinstalling_the_same_version_is_a_no_op() {
        let home = tempfile::tempdir().expect("home");
        let project = tempfile::tempdir().expect("project");
        install(
            &[Target::Codex],
            Scope::User,
            home.path(),
            project.path(),
            false,
        );
        let again = install(
            &[Target::Codex],
            Scope::User,
            home.path(),
            project.path(),
            false,
        );
        assert!(
            matches!(again[0].1, Outcome::Unchanged { .. }),
            "{:?}",
            again[0].1
        );
    }

    #[test]
    fn a_modified_skill_is_never_clobbered_without_force() {
        let home = tempfile::tempdir().expect("home");
        let project = tempfile::tempdir().expect("project");
        let dir = home.path().join(".claude/skills/sctxx");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("SKILL.md"), "# my own skill").expect("write");

        let results = install(
            &[Target::ClaudeCode],
            Scope::User,
            home.path(),
            project.path(),
            false,
        );
        assert!(
            matches!(results[0].1, Outcome::Modified { .. }),
            "{:?}",
            results[0].1
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("SKILL.md")).expect("read"),
            "# my own skill"
        );

        let forced = install(
            &[Target::ClaudeCode],
            Scope::User,
            home.path(),
            project.path(),
            true,
        );
        assert!(
            matches!(forced[0].1, Outcome::Installed { .. }),
            "{:?}",
            forced[0].1
        );
        assert!(
            std::fs::read_to_string(dir.join("SKILL.md"))
                .expect("read")
                .contains("sctxx")
        );
    }

    #[test]
    fn pi_has_no_project_scope_and_says_so_instead_of_failing() {
        let home = tempfile::tempdir().expect("home");
        let project = tempfile::tempdir().expect("project");
        let results = install(
            &[Target::Pi],
            Scope::Project,
            home.path(),
            project.path(),
            false,
        );
        assert!(
            matches!(results[0].1, Outcome::Skipped { .. }),
            "{:?}",
            results[0].1
        );
    }

    #[test]
    fn uninstall_removes_only_what_was_installed() {
        let home = tempfile::tempdir().expect("home");
        let project = tempfile::tempdir().expect("project");
        install(
            &[Target::Codex],
            Scope::User,
            home.path(),
            project.path(),
            false,
        );
        let removed = uninstall(&[Target::Codex], Scope::User, home.path(), project.path());
        assert!(
            matches!(removed[0].1, Outcome::Removed { .. }),
            "{:?}",
            removed[0].1
        );
        assert!(!home.path().join(".agents/skills/sctxx").exists());

        let again = uninstall(&[Target::Codex], Scope::User, home.path(), project.path());
        assert!(matches!(again[0].1, Outcome::Skipped { .. }));
    }

    #[test]
    fn the_skill_document_describes_the_trigger_and_the_commands() {
        assert!(SKILL_MD.starts_with("---"), "missing front matter");
        assert!(SKILL_MD.contains("name: sctxx"));
        assert!(SKILL_MD.contains("sctxx extract"));
        assert!(SKILL_MD.contains("sctxx expand"));
        assert!(CLI_REFERENCE.contains("sctxx extract"));
        assert!(ARTIFACT_REFERENCE.contains("L0"));
    }
}
