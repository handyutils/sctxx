//! LLM backends (spec §9).
//!
//! sctxx uses a model for semantic judgment only, and every backend is the
//! same shape: one text request in, one text response out, validated against a
//! JSON schema before it can change state.
//!
//! **Deviation from spec §4.3/§9 (decision D-4, `docs/adr/0001-...`).** The
//! spec proposed `reqwest` + `tokio`. The pipeline is sequential by design
//! (the fold must see chunk *k* before chunk *k+1*), so an async runtime would
//! buy nothing but a dependency tree and a colored-function boundary through
//! the whole crate. Backends are therefore blocking, and the one parallel
//! stage (premap) uses scoped OS threads.

pub mod api;
pub mod cli;
pub mod mock;
pub mod repair;

use crate::error::{Error, Result};

/// Which pipeline stage a request belongs to. Backends may map roles to
/// different models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CallRole {
    Premap,
    Fold,
    FinalPass,
}

impl CallRole {
    pub fn label(self) -> &'static str {
        match self {
            CallRole::Premap => "premap",
            CallRole::Fold => "fold",
            CallRole::FinalPass => "final_pass",
        }
    }
}

/// One model request.
#[derive(Debug, Clone)]
pub struct Request {
    pub role: CallRole,
    pub system: String,
    pub user: String,
    /// JSON Schema the response must satisfy, when the backend supports it.
    pub json_schema: Option<serde_json::Value>,
    pub max_output_tokens: u32,
    pub temperature: Option<f32>,
}

/// One model response.
#[derive(Debug, Clone, Default)]
pub struct Response {
    pub text: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// What a backend can do.
#[derive(Debug, Clone, Copy, Default)]
pub struct Capabilities {
    /// The provider enforces a JSON schema natively.
    pub json_schema_native: bool,
    /// Context window in tokens, when known.
    pub max_context: Option<usize>,
}

/// A source of model completions.
pub trait Backend: Send + Sync {
    /// Stable name, as it appears in the artifact header (`cli:claude`).
    fn name(&self) -> String;

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn complete(&self, request: &Request) -> Result<Response>;
}

/// A backend selection, as written on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Selection {
    /// Deterministic mode: no model is called at all.
    None,
    /// Choose the best available backend.
    Auto,
    /// A locally installed agent CLI used as a completion engine.
    Cli(String),
    /// An HTTP API: `anthropic`, `openai`, or `compat`, with an optional model.
    Api {
        provider: String,
        model: Option<String>,
    },
    /// Deterministic test double.
    Mock,
}

impl Selection {
    /// Parse a `--llm` value (spec §3.4).
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "none" => Ok(Selection::None),
            "auto" => Ok(Selection::Auto),
            "mock" => Ok(Selection::Mock),
            other => {
                if let Some(name) = other.strip_prefix("cli:") {
                    if name.is_empty() {
                        return Err(Error::Usage("cli: needs an agent name".into()));
                    }
                    return Ok(Selection::Cli(name.to_string()));
                }
                if let Some(rest) = other.strip_prefix("api:") {
                    let (provider, model) = match rest.split_once('/') {
                        Some((provider, model)) => (provider, Some(model.to_string())),
                        None => (rest, None),
                    };
                    if !matches!(provider, "anthropic" | "openai" | "compat") {
                        return Err(Error::Usage(format!(
                            "unknown api provider `{provider}` (expected anthropic, openai, or compat)"
                        )));
                    }
                    return Ok(Selection::Api {
                        provider: provider.to_string(),
                        model,
                    });
                }
                Err(Error::Usage(format!(
                    "unknown --llm value `{other}` (expected none, auto, cli:<name>, or api:<provider>[/<model>])"
                )))
            }
        }
    }
}

impl std::fmt::Display for Selection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Selection::None => write!(f, "none"),
            Selection::Auto => write!(f, "auto"),
            Selection::Mock => write!(f, "mock"),
            Selection::Cli(name) => write!(f, "cli:{name}"),
            Selection::Api { provider, model } => match model {
                Some(model) => write!(f, "api:{provider}/{model}"),
                None => write!(f, "api:{provider}"),
            },
        }
    }
}

/// Build a backend for a selection, resolving `auto` (spec §9.6).
///
/// Returns `Ok(None)` for [`Selection::None`], which is not an error: the
/// deterministic artifact is a supported product, not a degraded one.
pub fn build(selection: &Selection) -> Result<Option<Box<dyn Backend>>> {
    build_with_timeout(selection, cli::DEFAULT_TIMEOUT_SECS)
}

/// As [`build`], with an explicit timeout for the subprocess backends.
///
/// Only the `cli:` backends take it. `cli:codex` was measured killing a real
/// fold call at 600s and discarding the whole chunk, so it has to be adjustable
/// without editing the source.
pub fn build_with_timeout(
    selection: &Selection,
    timeout_secs: u64,
) -> Result<Option<Box<dyn Backend>>> {
    let timeout = std::time::Duration::from_secs(timeout_secs.max(1));
    match selection {
        Selection::None => Ok(None),
        Selection::Mock => Ok(Some(Box::new(mock::Mock::deterministic()))),
        Selection::Cli(name) => Ok(Some(Box::new(cli::CliBackend::with_timeout(
            name, timeout,
        )?))),
        Selection::Api { provider, model } => Ok(Some(Box::new(api::ApiBackend::new(
            provider,
            model.clone(),
        )?))),
        Selection::Auto => match resolve_auto() {
            Some(resolved) => build_with_timeout(&resolved, timeout_secs),
            None => Ok(None),
        },
    }
}

/// Resolution order for `auto`: an explicit environment override, then an API
/// key, then an installed agent CLI. `host` is never chosen automatically.
pub fn resolve_auto() -> Option<Selection> {
    if let Ok(value) = std::env::var("SCTXX_LLM")
        && let Ok(selection) = Selection::parse(&value)
        && selection != Selection::Auto
    {
        return Some(selection);
    }
    for (variable, provider) in api::KEY_ENV_VARS {
        if std::env::var_os(variable).is_some_and(|value| !value.is_empty()) {
            return Some(Selection::Api {
                provider: (*provider).to_string(),
                model: None,
            });
        }
    }
    for agent in cli::PREFERENCE_ORDER {
        if cli::find_executable(agent).is_some() {
            return Some(Selection::Cli((*agent).to_string()));
        }
    }
    None
}

/// Human-readable explanation of why `auto` found nothing, for stderr.
pub fn no_backend_hint() -> String {
    format!(
        "no LLM backend found: set one of {} for an API backend, install one of {} on PATH, \
         or pass --llm none for the deterministic artifact. `sctxx doctor` shows what was detected.",
        api::KEY_ENV_VARS
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", "),
        cli::PREFERENCE_ORDER.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selections_parse_and_render_round_trip() {
        for value in [
            "none",
            "auto",
            "mock",
            "cli:claude",
            "api:anthropic",
            "api:openai/gpt-5",
        ] {
            let selection = Selection::parse(value).expect(value);
            assert_eq!(selection.to_string(), value);
        }
    }

    #[test]
    fn unknown_selections_are_usage_errors() {
        for value in ["cli:", "api:bogus", "claude", ""] {
            let error = Selection::parse(value).expect_err(value);
            assert_eq!(error.exit_code(), 2, "{value}");
        }
    }

    #[test]
    fn none_builds_no_backend_and_that_is_not_an_error() {
        assert!(build(&Selection::None).expect("build").is_none());
    }

    #[test]
    fn mock_builds_a_usable_backend() {
        let backend = build(&Selection::Mock).expect("build").expect("backend");
        assert_eq!(backend.name(), "mock");
    }
}
