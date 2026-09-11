//! `sctxx update` — update an installed copy the way it was installed.
//!
//! sctxx ships through crates.io and npm, and those two are updated by
//! different tools. Rather than make the user remember which one they used,
//! sctxx looks at where it is running from: an npm install lives inside the npm
//! package directory, a `cargo install` lives in cargo's bin directory.
//!
//! Nothing is guessed silently. The detection, the reason, and the exact command
//! are printed before anything runs, and `--check` stops before the command.

use super::{GlobalArgs, out, out_json};
use crate::error::{Error, Result};
use clap::Args;
use std::path::{Path, PathBuf};
use std::process::Command;

/// `sctxx update`
#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Report how sctxx was installed and what updating would run, then stop.
    #[arg(long)]
    check: bool,
}

/// How this copy of sctxx got onto the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Installed by npm: the binary lives inside the npm package directory.
    Npm,
    /// Installed by `cargo install`: the binary lives in cargo's bin directory.
    Cargo,
    /// Some other install: a distribution package, a container, a checkout.
    Other,
}

impl Method {
    /// Which install a running executable belongs to.
    pub fn of(executable: Option<&Path>) -> Self {
        Self::with_cargo_bin(executable, cargo_bin_dir().as_deref())
    }

    /// The same decision with the cargo bin directory supplied, so it can be
    /// tested against a synthetic layout instead of the machine running it.
    fn with_cargo_bin(executable: Option<&Path>, cargo_bin: Option<&Path>) -> Self {
        let Some(executable) = executable else {
            return Method::Other;
        };
        // The npm wrapper runs `<...>/node_modules/sctxx-<platform>/bin/sctxx`.
        // Checked as a path component, not a substring, so a checkout that
        // happens to live under a directory called `npm-stuff` is not mistaken
        // for an npm install.
        if executable
            .components()
            .any(|part| part.as_os_str() == "node_modules")
        {
            return Method::Npm;
        }
        if let (Some(parent), Some(cargo_bin)) = (executable.parent(), cargo_bin)
            && parent == cargo_bin
        {
            return Method::Cargo;
        }
        Method::Other
    }

    pub fn name(self) -> &'static str {
        match self {
            Method::Npm => "npm",
            Method::Cargo => "cargo",
            Method::Other => "unknown",
        }
    }

    /// Why this method was chosen, for the reader who wants to disagree.
    pub fn why(self) -> &'static str {
        match self {
            Method::Npm => "the executable is inside a node_modules directory",
            Method::Cargo => "the executable is in cargo's bin directory",
            Method::Other => "the executable is in neither an npm nor a cargo location",
        }
    }

    /// The command that updates this install.
    pub fn plan(self) -> Result<Vec<String>> {
        match self {
            // `npm update` would be the obvious spelling, but it stays inside
            // the range recorded when the package was installed, so it cannot
            // cross a minor version. `@latest` means what it says.
            Method::Npm => Ok(vec![
                "npm".to_string(),
                "install".to_string(),
                "-g".to_string(),
                "sctxx@latest".to_string(),
            ]),
            // `cargo install` refuses to replace an existing binary without
            // `--force`, and replacing it is the entire point.
            Method::Cargo => Ok(vec![
                "cargo".to_string(),
                "install".to_string(),
                "sctxx".to_string(),
                "--force".to_string(),
            ]),
            Method::Other => Err(Error::Usage(format!(
                "sctxx was not installed by cargo or npm ({}), so it cannot update itself.\n\
                 Update it the way you installed it — a distribution package, a container image, \
                 or a build from a checkout.\n\
                 Both installers are available if you would rather switch:\n  \
                 cargo install sctxx\n  npm install -g sctxx@latest",
                self.why()
            ))),
        }
    }
}

/// `$CARGO_HOME/bin`, or `~/.cargo/bin`.
fn cargo_bin_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        return Some(PathBuf::from(home).join("bin"));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".cargo").join("bin"))
}

pub fn run(args: &UpdateArgs, global: &GlobalArgs) -> Result<i32> {
    let method = Method::of(std::env::current_exe().ok().as_deref());
    let command = method.plan()?;
    let shown = command.join(" ");

    global.note(&format!(
        "installed by: {} ({})",
        method.name(),
        method.why()
    ));

    if !args.check {
        global.note(&format!("running: {shown}"));
        // A fixed argv, never a shell: this is the developer's own package
        // manager, and nothing from a session is anywhere near it.
        let output = Command::new(&command[0])
            .args(&command[1..])
            .output()
            .map_err(|error| Error::Other(format!("could not run `{}`: {error}", command[0])))?;
        // The manager's own words go to stderr, so stdout stays the payload.
        forward(&output.stdout);
        forward(&output.stderr);
        if !output.status.success() {
            return Err(Error::Other(format!(
                "`{shown}` exited with {}",
                output.status.code().unwrap_or(-1)
            )));
        }
        global.note("done; `sctxx --version` reports the new one");
    }

    if global.json {
        out_json(&serde_json::json!({
            "method": method.name(),
            "why": method.why(),
            "command": command,
            "ran": !args.check,
        }))?;
    } else {
        out(&shown);
    }
    Ok(0)
}

/// Pass a child's stream through to our stderr, so it never reaches stdout.
fn forward(bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    use std::io::Write;
    let _ = std::io::stderr().write_all(bytes);
    let _ = std::io::stderr().flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_npm_install_is_recognised_wherever_node_modules_is() {
        for path in [
            "/usr/local/lib/node_modules/sctxx-darwin-arm64/bin/sctxx",
            "/home/dev/.nvm/versions/node/v22/lib/node_modules/sctxx-linux-x64/bin/sctxx",
        ] {
            assert_eq!(
                Method::of(Some(Path::new(path))),
                Method::Npm,
                "{path} should read as an npm install"
            );
        }
        // A Windows path is a single component when parsed on Unix, so this
        // assertion can only be made where the separators are real.
        #[cfg(windows)]
        assert_eq!(
            Method::of(Some(Path::new(
                "C:\\Users\\dev\\AppData\\Roaming\\npm\\node_modules\\sctxx-windows-arm64\\bin\\sctxx.exe"
            ))),
            Method::Npm
        );
    }

    #[test]
    fn a_cargo_install_is_recognised_by_cargo_bin() {
        let cargo_bin = Path::new("/Users/dev/.cargo/bin");
        assert_eq!(
            Method::with_cargo_bin(
                Some(Path::new("/Users/dev/.cargo/bin/sctxx")),
                Some(cargo_bin)
            ),
            Method::Cargo
        );
        // A different directory that merely ends in `bin` is not cargo's.
        assert_eq!(
            Method::with_cargo_bin(Some(Path::new("/usr/local/bin/sctxx")), Some(cargo_bin)),
            Method::Other
        );
    }

    #[test]
    fn a_checkout_build_is_not_mistaken_for_an_install() {
        // Neither installer owns `target/release`, so the honest answer is that
        // sctxx cannot update itself, not a guess at the wrong package manager.
        assert_eq!(
            Method::with_cargo_bin(
                Some(Path::new("/code/sctxx/target/release/sctxx")),
                Some(Path::new("/Users/dev/.cargo/bin"))
            ),
            Method::Other
        );
        assert_eq!(Method::of(None), Method::Other);
    }

    #[test]
    fn a_directory_merely_named_like_a_toolchain_is_not_an_install() {
        // Component-wise comparison, so a project path that mentions npm or
        // cargo does not decide how sctxx updates itself.
        assert_eq!(
            Method::with_cargo_bin(
                Some(Path::new("/code/npm-stuff/target/sctxx")),
                Some(Path::new("/Users/dev/.cargo/bin"))
            ),
            Method::Other
        );
        assert_eq!(
            Method::with_cargo_bin(
                Some(Path::new("/code/cargo-notes/sctxx")),
                Some(Path::new("/Users/dev/.cargo/bin"))
            ),
            Method::Other
        );
    }

    #[test]
    fn each_installer_gets_its_own_command() {
        assert_eq!(
            Method::Npm.plan().expect("npm has a plan").join(" "),
            "npm install -g sctxx@latest"
        );
        assert_eq!(
            Method::Cargo.plan().expect("cargo has a plan").join(" "),
            "cargo install sctxx --force"
        );
        // `cargo install` will not replace an existing binary without --force,
        // and replacing it is the whole point of update.
    }

    #[test]
    fn an_unknown_install_says_so_and_names_both_installers() {
        let error = Method::Other
            .plan()
            .expect_err("no plan for an unknown install");
        assert_eq!(error.exit_code(), 2);
        let message = error.to_string();
        assert!(message.contains("cargo install sctxx"), "{message}");
        assert!(message.contains("npm install -g sctxx@latest"), "{message}");
    }
}
