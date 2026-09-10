//! `sctxx redact` — strip secrets from a session file (spec §10.2).
//!
//! This ships before the adapters on purpose: a contributor cannot safely
//! share a fixture until there is a tool that scrubs one, and the project's own
//! test corpus depends on that being possible.

use super::{GlobalArgs, out, out_json};
use crate::error::{Error, Result};
use crate::vendor::codex::secrets::{RedactMode, redact, secret_classes};
use clap::Args;
use std::path::{Path, PathBuf};

/// `sctxx redact`
#[derive(Debug, Args)]
pub struct RedactArgs {
    /// File to redact. Use `-` for stdin.
    path: PathBuf,

    /// Also mask emails, private IPv4 addresses, and high-entropy strings.
    #[arg(long)]
    strict: bool,

    /// Write here instead of stdout.
    #[arg(long, value_name = "PATH")]
    out: Option<PathBuf>,

    /// Report what would be redacted without writing anything.
    #[arg(long)]
    check: bool,
}

pub fn run(args: &RedactArgs, global: &GlobalArgs) -> Result<i32> {
    let mode = if args.strict {
        RedactMode::Strict
    } else {
        RedactMode::Default
    };
    let input = if args.path == Path::new("-") {
        std::io::read_to_string(std::io::stdin()).map_err(|source| Error::io("<stdin>", source))?
    } else {
        std::fs::read_to_string(&args.path).map_err(|source| Error::io(&args.path, source))?
    };

    let redacted = redact(&input, mode);
    let occurrences =
        redacted.matches("[REDACTED_SECRET]").count() - input.matches("[REDACTED_SECRET]").count();

    if args.check {
        let report = serde_json::json!({
            "path": args.path,
            "mode": if args.strict { "strict" } else { "default" },
            "redactions": occurrences,
            "clean": occurrences == 0,
            "classes_checked": secret_classes(mode),
        });
        if global.json {
            out_json(&report)?;
        } else if occurrences == 0 {
            out(&format!(
                "{}: no secrets matched ({} classes checked)",
                args.path.display(),
                secret_classes(mode).len()
            ));
        } else {
            out(&format!(
                "{}: {occurrences} value(s) would be redacted",
                args.path.display()
            ));
        }
        return Ok(0);
    }

    match &args.out {
        Some(path) => {
            std::fs::write(path, &redacted).map_err(|source| Error::io(path, source))?;
            global.note(&format!(
                "wrote {} with {occurrences} redaction(s). Review it by hand before committing it \
                 as a fixture: redaction is pattern matching, not a guarantee.",
                path.display()
            ));
            out(&path.to_string_lossy());
        }
        None => {
            global.note(&format!("{occurrences} redaction(s)"));
            out(&redacted);
        }
    }
    Ok(0)
}
