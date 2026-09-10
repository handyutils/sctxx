//! `sctxx doctor` — what did sctxx detect on this machine?
//!
//! The first thing to run when something does not work: it names the store
//! directories that were searched, the backends that are available, and the
//! secret classes redaction covers. It never prints a key value.

use super::{GlobalArgs, out, out_json};
use crate::adapters::discovery;
use crate::error::Result;
use crate::ir::AgentKind;
use crate::vendor::codex::secrets::{RedactMode, secret_classes};

pub fn run(global: &GlobalArgs) -> Result<i32> {
    let roots = global.roots();
    let stores: Vec<serde_json::Value> = AgentKind::ALL
        .iter()
        .map(|agent| {
            let store = discovery::store(*agent, &roots);
            let sessions = discovery::list(*agent, &roots);
            serde_json::json!({
                "agent": agent.slug(),
                "label": agent.label(),
                "roots": store.roots.iter().map(|root| serde_json::json!({
                    "path": root,
                    "exists": root.is_dir(),
                })).collect::<Vec<_>>(),
                "sessions": sessions.len(),
                "newest": sessions.first().map(|session| session.reference()),
            })
        })
        .collect();

    let cli_backends: Vec<serde_json::Value> = crate::llm::cli::detected()
        .into_iter()
        .map(|(name, path)| serde_json::json!({ "backend": format!("cli:{name}"), "path": path }))
        .collect();
    let api_keys = crate::llm::api::detected_keys();
    let resolved = crate::llm::resolve_auto().map(|selection| selection.to_string());

    let features: Vec<&str> = [
        cfg!(feature = "zstd").then_some("zstd"),
        cfg!(feature = "api").then_some("api"),
        cfg!(feature = "cli-backends").then_some("cli-backends"),
    ]
    .into_iter()
    .flatten()
    .collect();

    let report = serde_json::json!({
        "sctxx": crate::VERSION,
        "features": features,
        "stores": stores,
        "llm": {
            "cli_backends": cli_backends,
            "api_keys_present": api_keys,
            "auto_resolves_to": resolved,
        },
        "redaction": {
            "default_classes": secret_classes(RedactMode::Default),
            "strict_classes": secret_classes(RedactMode::Strict),
        },
        "vendored_codex_commit": crate::vendor::codex::UPSTREAM_COMMIT,
    });

    if global.json {
        out_json(&report)?;
        return Ok(0);
    }

    let mut text = format!(
        "sctxx {}  ({})\n\nSession stores\n",
        crate::VERSION,
        features.join(", ")
    );
    for store in &stores {
        let agent = store["label"].as_str().unwrap_or_default();
        let count = store["sessions"].as_u64().unwrap_or(0);
        text.push_str(&format!("  {agent}: {count} session(s)\n"));
        if let Some(roots) = store["roots"].as_array() {
            for root in roots {
                let path = root["path"].as_str().unwrap_or_default();
                let exists = root["exists"].as_bool().unwrap_or(false);
                text.push_str(&format!(
                    "    {} {}\n",
                    if exists { "✓" } else { "–" },
                    path
                ));
            }
        }
    }

    text.push_str("\nLLM backends\n");
    if cli_backends.is_empty() {
        text.push_str("  no agent CLI found on PATH (looked for claude, codex, pi)\n");
    } else {
        for backend in &cli_backends {
            text.push_str(&format!(
                "  {} at {}\n",
                backend["backend"].as_str().unwrap_or_default(),
                backend["path"].as_str().unwrap_or_default()
            ));
        }
    }
    if api_keys.is_empty() {
        text.push_str("  no API key in the environment\n");
    } else {
        text.push_str(&format!(
            "  API keys present: {} (values never read here)\n",
            api_keys.join(", ")
        ));
    }
    text.push_str(&format!(
        "  --llm auto resolves to: {}\n",
        resolved.unwrap_or_else(|| "none (deterministic artifact)".to_string())
    ));

    text.push_str(&format!(
        "\nRedaction\n  {} secret classes by default, {} with --redact strict\n",
        secret_classes(RedactMode::Default).len(),
        secret_classes(RedactMode::Strict).len()
    ));
    text.push_str(&format!(
        "\nVendored from openai/codex at {}\n",
        crate::vendor::codex::UPSTREAM_COMMIT
    ));

    out(&text);
    Ok(0)
}
