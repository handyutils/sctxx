// Portions derived from OpenAI Codex (https://github.com/openai/codex),
// commit 818f1cca8ccf8899f0f4d59336baebaccf358eed, file
// codex-rs/utils/string/src/truncate.rs.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0.
// Modified by the sctxx authors: kept the UTF-8-safe middle-truncation and the
// 4-bytes-per-token estimate, dropped the char-count marker variant, and made
// the truncation marker stable for snapshot testing.

//! UTF-8-safe head+tail truncation and the shared token estimate.
//!
//! Every budget in sctxx is expressed in these approximate tokens so that a
//! single estimate governs masking, chunking, and artifact layers. Untrusted
//! text is only ever shortened through this module — never by byte slicing.

/// Codex's approximation: four bytes of UTF-8 text per model token.
const APPROX_BYTES_PER_TOKEN: usize = 4;

/// Approximate token count of `text`, rounding up.
pub fn approx_token_count(text: &str) -> usize {
    text.len().saturating_add(APPROX_BYTES_PER_TOKEN - 1) / APPROX_BYTES_PER_TOKEN
}

/// Byte budget that corresponds to `tokens` approximate tokens.
pub fn approx_bytes_for_tokens(tokens: usize) -> usize {
    tokens.saturating_mul(APPROX_BYTES_PER_TOKEN)
}

/// Truncate the middle of `s` to at most `max_bytes` bytes plus the marker,
/// keeping a prefix and a suffix and never splitting a `char`.
pub fn truncate_middle_bytes(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    if max_bytes == 0 {
        return truncation_marker(approx_token_count(s));
    }

    let (left_budget, right_budget) = split_budget(max_bytes);
    let (removed_bytes, before, after) = split_string(s, left_budget, right_budget);
    let mut out = String::with_capacity(before.len() + after.len() + 32);
    out.push_str(before);
    out.push_str(&truncation_marker(approx_token_count_of_bytes(
        removed_bytes,
    )));
    out.push_str(after);
    out
}

/// Truncate the middle of `s` to at most `max_tokens` approximate tokens.
pub fn truncate_middle_tokens(s: &str, max_tokens: usize) -> String {
    truncate_middle_bytes(s, approx_bytes_for_tokens(max_tokens))
}

/// Keep the first `max_bytes` bytes of `s` on a `char` boundary.
pub fn truncate_head_bytes(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let end = char_boundary_at_or_below(s, max_bytes);
    let mut out = String::with_capacity(end + 32);
    out.push_str(&s[..end]);
    out.push_str(&truncation_marker(approx_token_count_of_bytes(
        s.len() - end,
    )));
    out
}

/// Keep the last `max_bytes` bytes of `s` on a `char` boundary.
pub fn truncate_tail_bytes(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let start = char_boundary_at_or_above(s, s.len() - max_bytes);
    let mut out = String::with_capacity(max_bytes + 32);
    out.push_str(&truncation_marker(approx_token_count_of_bytes(start)));
    out.push_str(&s[start..]);
    out
}

fn truncation_marker(removed_tokens: usize) -> String {
    format!("…{removed_tokens} tokens truncated…")
}

fn approx_token_count_of_bytes(bytes: usize) -> usize {
    bytes.saturating_add(APPROX_BYTES_PER_TOKEN - 1) / APPROX_BYTES_PER_TOKEN
}

fn split_budget(budget: usize) -> (usize, usize) {
    let left = budget / 2;
    (left, budget - left)
}

fn char_boundary_at_or_below(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn char_boundary_at_or_above(s: &str, mut idx: usize) -> usize {
    while idx < s.len() && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

/// Split `s` into a prefix of at most `beginning_bytes` and a suffix of at most
/// `end_bytes`, both on `char` boundaries. Returns the number of bytes dropped.
fn split_string(s: &str, beginning_bytes: usize, end_bytes: usize) -> (usize, &str, &str) {
    if s.is_empty() {
        return (0, "", "");
    }
    let prefix_end = char_boundary_at_or_below(s, beginning_bytes);
    let suffix_start = char_boundary_at_or_above(s, s.len().saturating_sub(end_bytes));
    let suffix_start = suffix_start.max(prefix_end);
    (
        suffix_start - prefix_end,
        &s[..prefix_end],
        &s[suffix_start..],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_estimate_rounds_up() {
        assert_eq!(approx_token_count(""), 0);
        assert_eq!(approx_token_count("a"), 1);
        assert_eq!(approx_token_count("abcd"), 1);
        assert_eq!(approx_token_count("abcde"), 2);
        assert_eq!(approx_bytes_for_tokens(3), 12);
    }

    #[test]
    fn short_text_is_returned_unchanged() {
        assert_eq!(truncate_middle_bytes("hello", 5), "hello");
        assert_eq!(truncate_head_bytes("hello", 99), "hello");
        assert_eq!(truncate_tail_bytes("hello", 99), "hello");
    }

    #[test]
    fn middle_truncation_keeps_both_ends() {
        let out = truncate_middle_bytes("0123456789abcdefghij", 10);
        assert!(out.starts_with("01234"), "{out}");
        assert!(out.ends_with("fghij"), "{out}");
        assert!(out.contains("tokens truncated"), "{out}");
    }

    #[test]
    fn never_splits_a_multibyte_char() {
        // Every emoji is 4 bytes, so most budgets fall mid-character. Byte
        // slicing would panic; this must keep whole characters at every one.
        let s = "😀😀😀😀😀😀😀😀";
        for budget in 0..s.len() + 4 {
            for out in [
                truncate_middle_bytes(s, budget),
                truncate_head_bytes(s, budget),
                truncate_tail_bytes(s, budget),
            ] {
                assert!(
                    !out.contains('\u{fffd}'),
                    "replacement char at budget {budget}: {out}"
                );
                // The marker is delimited by ellipses; the rest is content.
                let content: String = out.split('…').step_by(2).collect();
                assert!(
                    content.chars().all(|c| c == '😀'),
                    "split a character at budget {budget}: {out}"
                );
            }
        }
    }

    #[test]
    fn truncation_output_respects_the_budget_plus_marker() {
        let s = "x".repeat(10_000);
        for budget in [1usize, 7, 64, 999] {
            let out = truncate_middle_bytes(&s, budget);
            let marker_len = truncation_marker(usize::MAX).len();
            assert!(
                out.len() <= budget + marker_len,
                "budget {budget}: {}",
                out.len()
            );
        }
    }
}
