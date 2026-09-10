// Portions derived from OpenAI Codex (https://github.com/openai/codex),
// commit 818f1cca8ccf8899f0f4d59336baebaccf358eed, file
// codex-rs/memories/write/src/rollout_input.rs (`serialize_tiered_input`).
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0.
// Modified by the sctxx authors: operates on sctxx masked rows instead of
// Codex `RolloutItem`s, adds the `PriorSummary` and `ToolResultError` tiers,
// and emits gap markers that carry the omitted event range so `sctxx expand`
// can recover them.

//! Tiered evidence budgeting.
//!
//! Codex's insight: when a whole transcript must fit one budget, filling it
//! newest-first *within a priority tier* keeps the highest-signal evidence
//! (human messages, then final assistant messages) even when tool output
//! dwarfs everything else. Rows are then rendered in source order so the
//! reader still sees a chronology, with explicit gap markers.

use crate::vendor::codex::truncate::{approx_bytes_for_tokens, truncate_middle_bytes};

/// Priority tiers, highest signal first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Tier {
    /// Human user messages and answers. Never dropped before anything else.
    User,
    /// Final assistant messages.
    AssistantFinal,
    /// Subagent spawn prompts and results.
    Subagent,
    /// Assistant commentary and readable reasoning.
    Commentary,
    /// Native compaction and branch summaries (low trust).
    PriorSummary,
    /// Failing tool results: the most useful tool output.
    ToolResultError,
    /// Tool calls.
    ToolCall,
    /// Successful tool results, usually already reduced to a placeholder.
    ToolResultOk,
}

impl Tier {
    /// Fill order used by [`select`].
    pub const ALL: [Tier; 8] = [
        Tier::User,
        Tier::AssistantFinal,
        Tier::Subagent,
        Tier::Commentary,
        Tier::PriorSummary,
        Tier::ToolResultError,
        Tier::ToolCall,
        Tier::ToolResultOk,
    ];
}

/// One budgetable unit of transcript.
#[derive(Debug, Clone)]
pub struct TieredRow {
    pub tier: Tier,
    /// Canonical event index this row came from.
    pub evt: u32,
    pub text: String,
}

/// A rendered selection: kept rows in source order, with gaps described.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    pub text: String,
    pub kept: usize,
    pub omitted: usize,
}

/// Reserve room for a truncation marker, as upstream does.
const TRUNCATION_RESERVE_BYTES: usize = 96;

/// Select rows newest-first within each tier under `token_limit`, then render
/// them in source order with `[... N events omitted (evt a-b) ...]` gaps.
pub fn select(rows: &[TieredRow], token_limit: usize) -> Selection {
    let budget = approx_bytes_for_tokens(token_limit);
    let mut remaining = budget;
    let mut selected: Vec<Option<String>> = vec![None; rows.len()];

    for tier in Tier::ALL {
        for (index, row) in rows.iter().enumerate().rev() {
            if row.tier != tier {
                continue;
            }
            let text = if row.text.len() <= remaining {
                // Fits whole: no marker needed, so no reserve needed either.
                row.text.clone()
            } else if remaining > TRUNCATION_RESERVE_BYTES {
                truncate_middle_bytes(&row.text, remaining - TRUNCATION_RESERVE_BYTES)
            } else {
                // Too little left to say anything useful; leave a gap instead
                // of a stub. A short high-tier row can still fit later.
                continue;
            };
            remaining = remaining.saturating_sub(text.len());
            selected[index] = Some(text);
        }
    }

    let mut out = Selection::default();
    let mut gap_start: Option<u32> = None;
    let mut gap_end: u32 = 0;
    let mut gap_count = 0usize;

    let flush_gap =
        |out: &mut Selection, gap_start: &mut Option<u32>, gap_end: u32, count: &mut usize| {
            if let Some(start) = gap_start.take() {
                out.text.push_str(&format!(
                    "[... {count} events omitted (evt {start}-{gap_end}) ...]\n"
                ));
                out.omitted += *count;
                *count = 0;
            }
        };

    for (index, row) in rows.iter().enumerate() {
        match &selected[index] {
            Some(text) => {
                flush_gap(&mut out, &mut gap_start, gap_end, &mut gap_count);
                out.text.push_str(text);
                if !text.ends_with('\n') {
                    out.text.push('\n');
                }
                out.kept += 1;
            }
            None => {
                if gap_start.is_none() {
                    gap_start = Some(row.evt);
                }
                gap_end = row.evt;
                gap_count += 1;
            }
        }
    }
    flush_gap(&mut out, &mut gap_start, gap_end, &mut gap_count);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(tier: Tier, evt: u32, text: &str) -> TieredRow {
        TieredRow {
            tier,
            evt,
            text: text.to_string(),
        }
    }

    #[test]
    fn everything_fits_under_a_large_budget() {
        let rows = vec![
            row(Tier::User, 0, "[user] hi"),
            row(Tier::ToolCall, 1, "[call Read]"),
        ];
        let out = select(&rows, 1_000);
        assert_eq!(out.kept, 2);
        assert_eq!(out.omitted, 0);
        assert!(out.text.contains("[user] hi"));
        assert!(!out.text.contains("omitted"));
    }

    #[test]
    fn user_rows_outlive_tool_rows_under_pressure() {
        let rows = vec![
            row(Tier::ToolResultOk, 0, &"t".repeat(400)),
            row(Tier::User, 1, "[user] keep me"),
            row(Tier::ToolResultOk, 2, &"t".repeat(400)),
        ];
        // 32 bytes: the user row fits whole, neither tool row can say anything.
        let out = select(&rows, 8);
        assert!(out.text.contains("keep me"), "{}", out.text);
        assert_eq!(out.omitted, 2, "{}", out.text);
    }

    #[test]
    fn gap_markers_carry_the_omitted_event_range() {
        let rows = vec![
            row(Tier::ToolResultOk, 10, &"t".repeat(400)),
            row(Tier::ToolResultOk, 11, &"t".repeat(400)),
            row(Tier::User, 12, "[user] keep"),
        ];
        let out = select(&rows, 8);
        assert!(out.text.contains("(evt 10-11)"), "{}", out.text);
    }

    #[test]
    fn a_tool_row_is_truncated_rather_than_dropped_when_there_is_room() {
        let rows = vec![row(Tier::ToolResultOk, 0, &"t".repeat(4_000))];
        let out = select(&rows, 200);
        assert_eq!(out.kept, 1);
        assert!(out.text.contains("tokens truncated"), "{}", out.text);
    }

    #[test]
    fn rows_render_in_source_order_not_tier_order() {
        let rows = vec![
            row(Tier::ToolCall, 0, "[call first]"),
            row(Tier::User, 1, "[user] second"),
        ];
        let out = select(&rows, 1_000);
        let call_at = out.text.find("first").unwrap_or(usize::MAX);
        let user_at = out.text.find("second").unwrap_or(0);
        assert!(call_at < user_at, "{}", out.text);
    }

    #[test]
    fn selection_never_exceeds_the_budget_by_more_than_the_gap_markers() {
        let rows: Vec<_> = (0..50)
            .map(|i| row(Tier::ToolResultOk, i, &"x".repeat(200)))
            .collect();
        for limit in [1usize, 10, 100, 1_000] {
            let out = select(&rows, limit);
            let body: usize = out
                .text
                .lines()
                .filter(|l| !l.starts_with("[..."))
                .map(str::len)
                .sum();
            assert!(
                body <= approx_bytes_for_tokens(limit) + 1,
                "limit {limit}: {body}"
            );
        }
    }
}
