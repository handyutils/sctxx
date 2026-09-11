//! `api:<provider>` backends (spec §9.2), behind the `api` feature.
//!
//! Blocking HTTP through `ureq` (decision D-4). Three providers cover the
//! field: Anthropic's Messages API, OpenAI's Chat Completions API, and any
//! OpenAI-compatible endpoint (OpenRouter, DeepSeek, Ollama, vLLM, LM Studio),
//! which is why `compat` exists rather than one backend per vendor.

use super::{Backend, Request, Response};
// Only the feature-gated `Backend` impl advertises capabilities; without the
// feature the stub below needs none of it.
#[cfg(feature = "api")]
use super::Capabilities;
use crate::error::{Error, Result};

/// Environment variables that indicate a usable API backend, in `auto` order.
pub const KEY_ENV_VARS: &[(&str, &str)] = &[
    ("ANTHROPIC_API_KEY", "anthropic"),
    ("OPENAI_API_KEY", "openai"),
    ("SCTXX_API_KEY", "compat"),
];

/// Default models, chosen for "good enough to fold, cheap enough to run".
/// Override with `--llm api:<provider>/<model>`.
const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-5";
const DEFAULT_OPENAI_MODEL: &str = "gpt-4.1-mini";

/// An HTTP LLM backend.
#[derive(Debug)]
pub struct ApiBackend {
    provider: String,
    model: String,
    key: String,
    base_url: String,
}

impl ApiBackend {
    /// Resolve credentials and the endpoint for `provider`.
    pub fn new(provider: &str, model: Option<String>) -> Result<Self> {
        let (key_var, default_model, default_base) = match provider {
            "anthropic" => (
                "ANTHROPIC_API_KEY",
                DEFAULT_ANTHROPIC_MODEL,
                "https://api.anthropic.com",
            ),
            "openai" => (
                "OPENAI_API_KEY",
                DEFAULT_OPENAI_MODEL,
                "https://api.openai.com",
            ),
            "compat" => ("SCTXX_API_KEY", "", ""),
            other => {
                return Err(Error::Usage(format!("unknown api provider `{other}`")));
            }
        };

        let key = std::env::var(key_var).unwrap_or_default();
        if key.is_empty() && provider != "compat" {
            return Err(Error::LlmUnavailable(format!("{key_var} is not set")));
        }
        let base_url = std::env::var("SCTXX_BASE_URL").unwrap_or_else(|_| default_base.to_string());
        if base_url.is_empty() {
            return Err(Error::LlmUnavailable(
                "api:compat needs SCTXX_BASE_URL (and usually SCTXX_API_KEY)".into(),
            ));
        }
        let model = model
            .or_else(|| std::env::var("SCTXX_MODEL").ok())
            .unwrap_or_else(|| default_model.to_string());
        if model.is_empty() {
            return Err(Error::LlmUnavailable(
                "api:compat needs a model: --llm api:compat/<model> or SCTXX_MODEL".into(),
            ));
        }
        Ok(Self {
            provider: provider.to_string(),
            model,
            key,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }
}

#[cfg(feature = "api")]
impl Backend for ApiBackend {
    fn name(&self) -> String {
        format!("api:{}/{}", self.provider, self.model)
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            json_schema_native: false,
            max_context: None,
        }
    }

    fn complete(&self, request: &Request) -> Result<Response> {
        let (url, body) = match self.provider.as_str() {
            "anthropic" => (
                format!("{}/v1/messages", self.base_url),
                serde_json::json!({
                    "model": self.model,
                    "max_tokens": request.max_output_tokens,
                    "system": request.system,
                    "temperature": request.temperature.unwrap_or(0.0),
                    "messages": [{"role": "user", "content": request.user}],
                }),
            ),
            _ => (
                format!("{}/v1/chat/completions", self.base_url),
                serde_json::json!({
                    "model": self.model,
                    "max_completion_tokens": request.max_output_tokens,
                    "temperature": request.temperature.unwrap_or(0.0),
                    "messages": [
                        {"role": "system", "content": request.system},
                        {"role": "user", "content": request.user},
                    ],
                }),
            ),
        };

        let value = self.post_with_retries(&url, &body)?;
        let text = extract_text(&self.provider, &value).ok_or_else(|| Error::LlmFailed {
            backend: self.name(),
            message: format!(
                "no completion in the response: {}",
                crate::vendor::codex::truncate::truncate_middle_bytes(&value.to_string(), 400)
            ),
        })?;
        let (input_tokens, output_tokens) = extract_usage(&value);
        Ok(Response {
            text,
            input_tokens,
            output_tokens,
        })
    }
}

#[cfg(feature = "api")]
impl ApiBackend {
    /// POST with exponential backoff on 429 and 5xx (spec §9.2).
    fn post_with_retries(&self, url: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        const MAX_ATTEMPTS: u32 = 4;
        let mut last_error = String::new();
        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(500 << attempt));
            }
            let mut post = ureq::post(url)
                .set("content-type", "application/json")
                .timeout(std::time::Duration::from_secs(600));
            post = match self.provider.as_str() {
                "anthropic" => post
                    .set("x-api-key", &self.key)
                    .set("anthropic-version", "2023-06-01"),
                _ if self.key.is_empty() => post,
                _ => post.set("authorization", &format!("Bearer {}", self.key)),
            };

            match post.send_json(body.clone()) {
                Ok(response) => {
                    return response.into_json::<serde_json::Value>().map_err(|error| {
                        Error::LlmFailed {
                            backend: self.name(),
                            message: format!("response was not JSON: {error}"),
                        }
                    });
                }
                Err(ureq::Error::Status(status, response)) => {
                    let detail = response.into_string().unwrap_or_default();
                    last_error = format!(
                        "HTTP {status}: {}",
                        crate::vendor::codex::truncate::truncate_middle_bytes(detail.trim(), 300)
                    );
                    let retryable = status == 429 || (500..600).contains(&status);
                    if !retryable {
                        break;
                    }
                }
                Err(error) => {
                    last_error = error.to_string();
                }
            }
        }
        Err(Error::LlmFailed {
            backend: self.name(),
            message: last_error,
        })
    }
}

/// Without the `api` feature the type still exists so `--llm api:…` can fail
/// with a clear message instead of an unknown flag.
#[cfg(not(feature = "api"))]
impl Backend for ApiBackend {
    fn name(&self) -> String {
        format!("api:{}/{}", self.provider, self.model)
    }

    fn complete(&self, _request: &Request) -> Result<Response> {
        let _ = (&self.key, &self.base_url);
        Err(Error::LlmUnavailable(
            "this build was compiled without the `api` feature; use --llm cli:<agent> or --llm none"
                .into(),
        ))
    }
}

/// Pull the completion text out of either response shape.
#[cfg(feature = "api")]
fn extract_text(provider: &str, value: &serde_json::Value) -> Option<String> {
    if provider == "anthropic" {
        let blocks = value.get("content")?.as_array()?;
        let text: String = blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(|text| text.as_str()))
            .collect::<Vec<&str>>()
            .join("");
        return (!text.is_empty()).then_some(text);
    }
    let choices = value.get("choices")?.as_array()?;
    let text = choices
        .first()?
        .get("message")?
        .get("content")?
        .as_str()
        .map(str::to_string)?;
    (!text.is_empty()).then_some(text)
}

#[cfg(feature = "api")]
fn extract_usage(value: &serde_json::Value) -> (Option<u64>, Option<u64>) {
    let usage = value.get("usage");
    let input = usage
        .and_then(|usage| {
            usage
                .get("input_tokens")
                .or_else(|| usage.get("prompt_tokens"))
        })
        .and_then(serde_json::Value::as_u64);
    let output = usage
        .and_then(|usage| {
            usage
                .get("output_tokens")
                .or_else(|| usage.get("completion_tokens"))
        })
        .and_then(serde_json::Value::as_u64);
    (input, output)
}

/// API keys present in the environment, for `sctxx doctor`. Never the values.
pub fn detected_keys() -> Vec<&'static str> {
    KEY_ENV_VARS
        .iter()
        .filter(|(variable, _)| std::env::var_os(variable).is_some_and(|value| !value.is_empty()))
        .map(|(variable, _)| *variable)
        .collect()
}

/// These exercise the response parsing that only the `api` feature compiles,
/// so they are gated with it rather than left to fail on a minimal build.
#[cfg(all(test, feature = "api"))]
mod tests {
    use super::*;

    #[test]
    fn anthropic_responses_are_read_from_content_blocks() {
        let value = serde_json::json!({
            "content": [{"type": "text", "text": "{\"ops\":[]}"}],
            "usage": {"input_tokens": 10, "output_tokens": 4}
        });
        assert_eq!(
            extract_text("anthropic", &value).as_deref(),
            Some("{\"ops\":[]}")
        );
        assert_eq!(extract_usage(&value), (Some(10), Some(4)));
    }

    #[test]
    fn openai_compatible_responses_are_read_from_choices() {
        let value = serde_json::json!({
            "choices": [{"message": {"content": "hello"}}],
            "usage": {"prompt_tokens": 7, "completion_tokens": 2}
        });
        assert_eq!(extract_text("openai", &value).as_deref(), Some("hello"));
        assert_eq!(extract_usage(&value), (Some(7), Some(2)));
    }

    #[test]
    fn an_unexpected_shape_yields_none_rather_than_a_panic() {
        let value = serde_json::json!({"error": {"message": "bad request"}});
        assert!(extract_text("openai", &value).is_none());
        assert!(extract_text("anthropic", &value).is_none());
        assert_eq!(extract_usage(&value), (None, None));
    }

    #[test]
    fn a_missing_key_makes_the_backend_unavailable_not_a_usage_error() {
        // Only meaningful when the developer has no key set.
        if std::env::var_os("ANTHROPIC_API_KEY").is_none() {
            let error = ApiBackend::new("anthropic", None).expect_err("should be unavailable");
            assert_eq!(error.exit_code(), 6);
        }
    }

    #[test]
    fn an_unknown_provider_is_a_usage_error() {
        assert_eq!(
            ApiBackend::new("bogus", None)
                .expect_err("reject")
                .exit_code(),
            2
        );
    }
}
