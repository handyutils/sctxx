// Portions derived from OpenAI Codex (https://github.com/openai/codex),
// commit 818f1cca8ccf8899f0f4d59336baebaccf358eed, file
// codex-rs/secrets/src/sanitizer.rs.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0.
// Modified by the sctxx authors: kept the four upstream patterns and the
// [REDACTED_SECRET] replacement token, added the secret classes required by
// docs/SCTXX-SPEC.md §10.2, added a strict mode, and replaced the panicking
// regex constructor with a compile-time-checked table.

//! Best-effort secret redaction.
//!
//! Applied to every masked row before it reaches an LLM backend, to every LLM
//! response, and to every rendered artifact. Redaction is a safety net, not a
//! guarantee: it can only remove what it recognizes.

use regex::Regex;
use std::sync::LazyLock;

/// The replacement token, kept identical to Codex so artifacts read the same.
pub const REDACTED: &str = "[REDACTED_SECRET]";

/// How aggressively to redact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum RedactMode {
    /// No redaction. Only honored for `--llm none` and host mode.
    Off,
    /// Known secret shapes (spec §10.2).
    #[default]
    Default,
    /// Also emails, private-range IPv4 addresses, and high-entropy strings.
    Strict,
}

struct Pattern {
    name: &'static str,
    /// `None` only if the pattern failed to compile, which
    /// `every_pattern_compiles` turns into a test failure. Redaction skips such
    /// a pattern rather than panicking in the egress path.
    regex: Option<Regex>,
    /// Replacement template; `$1`-style groups keep the recognizable prefix.
    replacement: &'static str,
    strict_only: bool,
}

macro_rules! pattern {
    ($name:literal, $re:literal, $repl:literal, $strict:literal) => {
        Pattern {
            name: $name,
            regex: Regex::new($re).ok(),
            replacement: $repl,
            strict_only: $strict,
        }
    };
}

/// The pattern table, compiled once on first use.
fn patterns() -> &'static [Pattern] {
    &PATTERNS
}

static PATTERNS: LazyLock<Vec<Pattern>> = LazyLock::new(|| {
    vec![
        // ---- upstream Codex patterns -------------------------------------------
        pattern!(
            "bearer_token",
            r"(?i:\bBearer)[ \t]+[A-Za-z0-9._~+/-]{16,}=*",
            "Bearer [REDACTED_SECRET]",
            false
        ),
        pattern!(
            "openai_key",
            r"sk-[A-Za-z0-9_-]{20,}",
            "[REDACTED_SECRET]",
            false
        ),
        pattern!(
            "aws_access_key_id",
            r"\bAKIA[0-9A-Z]{16}\b",
            "[REDACTED_SECRET]",
            false
        ),
        // ---- added for sctxx (spec §10.2) --------------------------------------
        pattern!(
            "anthropic_key",
            r"sk-ant-[A-Za-z0-9_-]{20,}",
            "[REDACTED_SECRET]",
            false
        ),
        pattern!(
            "github_token",
            r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,}\b|\bgithub_pat_[A-Za-z0-9_]{22,}\b",
            "[REDACTED_SECRET]",
            false
        ),
        pattern!(
            "slack_token",
            r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b",
            "[REDACTED_SECRET]",
            false
        ),
        pattern!(
            "stripe_key",
            r"\b(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{16,}\b",
            "[REDACTED_SECRET]",
            false
        ),
        pattern!(
            "google_api_key",
            r"\bAIza[A-Za-z0-9_-]{35}\b",
            "[REDACTED_SECRET]",
            false
        ),
        pattern!(
            "jwt",
            r"\beyJ[A-Za-z0-9_-]{6,}\.[A-Za-z0-9_-]{6,}\.[A-Za-z0-9_-]{6,}\b",
            "[REDACTED_SECRET]",
            false
        ),
        pattern!(
            "pem_private_key",
            r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
            "[REDACTED_SECRET]",
            false
        ),
        pattern!(
            "connection_string_password",
            r"([a-zA-Z][a-zA-Z0-9+.-]*://[^\s:/@]+):[^\s/@]+@",
            "$1:[REDACTED_SECRET]@",
            false
        ),
        // Assignments come last so a more specific pattern above wins first.
        pattern!(
            "secret_assignment",
            r#"(?i)\b(api[_-]?key|secret[_-]?key|access[_-]?token|refresh[_-]?token|client[_-]?secret|token|secret|password|passwd|private[_-]?key)\b(\s*[:=]\s*)(["']?)[^\s"',]{8,}"#,
            "$1$2$3[REDACTED_SECRET]",
            false
        ),
        // ---- strict mode -------------------------------------------------------
        pattern!(
            "email",
            r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b",
            "[REDACTED_SECRET]",
            true
        ),
        pattern!(
            "private_ipv4",
            r"\b(?:10\.\d{1,3}\.\d{1,3}\.\d{1,3}|192\.168\.\d{1,3}\.\d{1,3}|172\.(?:1[6-9]|2\d|3[01])\.\d{1,3}\.\d{1,3})\b",
            "[REDACTED_SECRET]",
            true
        ),
        pattern!(
            "high_entropy",
            r"\b[A-Za-z0-9+/_-]{32,}={0,2}\b",
            "[REDACTED_SECRET]",
            true
        ),
    ]
});

/// Names of every secret class this module recognizes, for `doctor` and tests.
pub fn secret_classes(mode: RedactMode) -> Vec<&'static str> {
    patterns()
        .iter()
        .filter(|p| match mode {
            RedactMode::Off => false,
            RedactMode::Default => !p.strict_only,
            RedactMode::Strict => true,
        })
        .map(|p| p.name)
        .collect()
}

/// Redact known secret shapes from `input`.
pub fn redact(input: &str, mode: RedactMode) -> String {
    if mode == RedactMode::Off {
        return input.to_string();
    }
    let mut out = input.to_string();
    for pattern in patterns() {
        if pattern.strict_only && mode != RedactMode::Strict {
            continue;
        }
        if let Some(regex) = &pattern.regex {
            out = regex.replace_all(&out, pattern.replacement).into_owned();
        }
    }
    out
}

/// True when `input` still contains something this module would redact.
pub fn contains_secret(input: &str, mode: RedactMode) -> bool {
    redact(input, mode) != input
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_compiles() {
        for pattern in patterns() {
            assert!(
                pattern.regex.is_some(),
                "{} failed to compile",
                pattern.name
            );
        }
    }

    #[test]
    fn redacts_upstream_codex_classes() {
        assert_eq!(
            redact(
                "Bearer abcde+fghijklmnopqrstuvwxyz012345",
                RedactMode::Default
            ),
            "Bearer [REDACTED_SECRET]"
        );
        assert_eq!(
            redact("key=sk-abcdefghijklmnopqrstuvwxyz", RedactMode::Default),
            "key=[REDACTED_SECRET]"
        );
        assert_eq!(
            redact("AKIAIOSFODNN7EXAMPLE", RedactMode::Default),
            "[REDACTED_SECRET]"
        );
    }

    #[test]
    fn redacts_every_added_secret_class() {
        // A real Google API key is `AIza` plus exactly 35 characters.
        let google = format!("AIza{}", "A".repeat(35));
        let cases = [
            "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAA",
            "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
            "github_pat_AAAAAAAAAAAAAAAAAAAAAA_BBBB",
            "xoxb-1234567890-abcdefghij",
            "sk_live_ABCDEFGHIJKLMNOPQRST",
            google.as_str(),
            "eyJhbGciOi.eyJzdWIiOi.SflKxwRJSM",
            "-----BEGIN RSA PRIVATE KEY-----\nabc\n-----END RSA PRIVATE KEY-----",
            "postgres://admin:sup3rs3cret@db.internal:5432/app",
            "PASSWORD=hunter2hunter2",
            "api_key: 0123456789abcdef",
        ];
        for case in cases {
            assert!(
                contains_secret(case, RedactMode::Default),
                "not redacted in default mode: {case}"
            );
            assert!(
                !redact(case, RedactMode::Default).contains("sup3rs3cret"),
                "password survived: {case}"
            );
        }
    }

    #[test]
    fn connection_string_keeps_scheme_and_user() {
        assert_eq!(
            redact(
                "postgres://admin:sup3rs3cret@db:5432/app",
                RedactMode::Default
            ),
            "postgres://admin:[REDACTED_SECRET]@db:5432/app"
        );
    }

    #[test]
    fn strict_mode_adds_emails_ips_and_entropy() {
        let text = "mail dev@example.com from 10.0.0.7";
        assert!(
            !contains_secret(text, RedactMode::Default),
            "default mode over-redacted"
        );
        assert!(contains_secret(text, RedactMode::Strict));
        assert!(
            secret_classes(RedactMode::Strict).len() > secret_classes(RedactMode::Default).len()
        );
        assert!(secret_classes(RedactMode::Off).is_empty());
    }

    #[test]
    fn leaves_ordinary_prose_alone() {
        for text in [
            "Bearer of good news",
            "the token was invalid",
            "run cargo test --all-features",
            "src/auth.ts imports jsonwebtoken",
        ] {
            assert_eq!(
                redact(text, RedactMode::Default),
                text,
                "over-redacted: {text}"
            );
        }
    }

    #[test]
    fn redaction_is_idempotent() {
        let once = redact("token=abcdefghijklmnop", RedactMode::Default);
        assert_eq!(redact(&once, RedactMode::Default), once);
    }
}
