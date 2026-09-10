//! Library error type and the process exit codes it maps to.
//!
//! Exit codes are a public contract (`docs/SCTXX-SPEC.md` §3.1): agents branch
//! on them, so they change only with a version decision.

use std::path::PathBuf;

/// Every way a sctxx operation can fail.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("usage: {0}")]
    Usage(String),

    #[error("session not found: {0}")]
    SessionNotFound(String),

    /// More than one session matched. The candidates are printed as JSON so a
    /// calling agent can show them to the user.
    #[error("ambiguous session reference `{reference}`: {count} candidates")]
    Ambiguous {
        reference: String,
        count: usize,
        candidates_json: String,
    },

    #[error("could not detect the agent that wrote {0}")]
    UnknownFormat(PathBuf),

    #[error("{path}: {bad} of {total} lines failed to parse ({rate:.1}% > {max:.1}% allowed)")]
    ParseFailureRate {
        path: PathBuf,
        bad: usize,
        total: usize,
        rate: f64,
        max: f64,
    },

    #[error("no usable LLM backend: {0}")]
    LlmUnavailable(String),

    #[error("LLM backend `{backend}` failed: {message}")]
    LlmFailed { backend: String, message: String },

    #[error("verification found {0} contradiction(s) and --strict was set")]
    Contradicted(usize),

    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Wrap an I/O error with the path it happened on.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    /// The process exit code for this error (spec §3.1).
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Usage(_) => 2,
            Error::Ambiguous { .. } => 3,
            Error::SessionNotFound(_) => 4,
            Error::UnknownFormat(_) | Error::ParseFailureRate { .. } => 5,
            Error::LlmUnavailable(_) | Error::LlmFailed { .. } => 6,
            Error::Contradicted(_) => 7,
            Error::Io { .. } | Error::Other(_) => 1,
        }
    }
}

/// Result alias used throughout the library.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_the_published_contract() {
        assert_eq!(Error::Usage("x".into()).exit_code(), 2);
        assert_eq!(
            Error::Ambiguous {
                reference: "a".into(),
                count: 2,
                candidates_json: "[]".into()
            }
            .exit_code(),
            3
        );
        assert_eq!(Error::SessionNotFound("a".into()).exit_code(), 4);
        assert_eq!(Error::UnknownFormat(PathBuf::from("a")).exit_code(), 5);
        assert_eq!(Error::LlmUnavailable("a".into()).exit_code(), 6);
        assert_eq!(Error::Contradicted(1).exit_code(), 7);
        assert_eq!(Error::Other("a".into()).exit_code(), 1);
    }
}
