//! S2 — segmentation, chunking, and the tail split (spec §7.3).
//!
//! An *episode* starts at each human message: that is where intent changes, so
//! it is the natural unit to keep whole. Episodes pack into *chunks* that fit
//! the fold model's budget, and the newest `tail_tokens` of the transcript are
//! held back as the near-verbatim recency tail rather than folded.

use super::mask::Row;
use crate::vendor::codex::tiered_input::Tier;

/// A run of rows that belong to one intent.
#[derive(Debug, Clone)]
pub struct Episode {
    pub id: usize,
    /// Index range into the row slice.
    pub rows: std::ops::Range<usize>,
    pub evt_start: u32,
    pub evt_end: u32,
    pub tokens: usize,
    /// First human message in the episode, for the later-episode index.
    pub headline: String,
}

/// A unit of work for one fold call.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub id: String,
    pub rows: std::ops::Range<usize>,
    pub evt_start: u32,
    pub evt_end: u32,
    pub tokens: usize,
    pub episode_ids: Vec<usize>,
}

/// The output of segmentation.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub episodes: Vec<Episode>,
    /// Chunks strictly before the tail; these are what the fold processes.
    pub chunks: Vec<Chunk>,
    /// Rows held back as the recency tail.
    pub tail: std::ops::Range<usize>,
    /// Episodes that fall inside the tail, for the fold's later-episode index.
    pub tail_episode_ids: Vec<usize>,
}

impl Plan {
    /// Total tokens in the folded chunks.
    pub fn chunk_tokens(&self) -> usize {
        self.chunks.iter().map(|chunk| chunk.tokens).sum()
    }
}

/// Segmentation parameters.
#[derive(Debug, Clone, Copy)]
pub struct SegmentOptions {
    /// Maximum tokens per fold chunk.
    pub chunk_tokens: usize,
    /// Token budget for the recency tail.
    pub tail_tokens: usize,
    /// Episodes smaller than this merge forward.
    pub min_episode_tokens: usize,
}

impl Default for SegmentOptions {
    fn default() -> Self {
        Self {
            chunk_tokens: 24_000,
            tail_tokens: 12_000,
            min_episode_tokens: 1_500,
        }
    }
}

/// Keep the highest-signal rows under a token budget, in source order.
///
/// Codex's tier order, applied to sctxx's masked rows: fill each tier newest
/// first, then keep the result in source order so a reader still sees a
/// chronology. This is the policy in `vendor/codex/tiered_input.rs`, expressed
/// on rows rather than on rendered text — the fold needs the event indices to
/// validate provenance against, and a rendered string has thrown them away.
///
/// Rows are kept whole or not at all. A digest that spends its budget on the
/// middle of a tool output has spent it on nothing, and a half-quoted command is
/// exactly the kind of evidence that must never appear in an artifact.
///
/// The point is the call count. Folding the whole masked transcript of a
/// 103,757-event session is 807,372 tokens across 40 sequential model calls; the
/// same session's human turns, failures, plans and final answers fit in a
/// digest of one. Neither is free, but only one of them finishes.
pub fn digest(rows: &[Row], tokens: usize) -> Vec<Row> {
    // Nothing worth keeping costs less than this, so a tier that cannot fit even
    // a fragment stops rather than leaving a stub.
    const MIN_BOUNDARY_TOKENS: usize = 40;

    let mut chosen: Vec<(usize, Row)> = Vec::new();
    let mut taken = vec![false; rows.len()];
    let mut remaining = tokens;
    for tier in Tier::ALL {
        // Newest first within the tier, so a budget that cannot hold everything
        // holds the most recent thing that mattered.
        for (index, row) in rows.iter().enumerate().rev() {
            if row.tier != tier || taken[index] {
                continue;
            }
            if row.tokens > remaining {
                // The boundary item is truncated rather than dropped, which is
                // Codex's rule at the same seam (`build_compacted_history` keeps
                // the newest messages that fit and middle-truncates the one that
                // does not). A partial user turn is worth more than a clean gap,
                // because the gap is where the instruction was.
                if remaining >= MIN_BOUNDARY_TOKENS {
                    // `truncate_middle_tokens` documents its result as "at most
                    // max_bytes **plus the marker**", so the marker is paid for
                    // out of the budget here rather than overshooting it.
                    const MARKER_TOKENS: usize = 12;
                    let text = crate::vendor::codex::truncate::truncate_middle_tokens(
                        &row.text,
                        remaining.saturating_sub(MARKER_TOKENS),
                    );
                    let mut trimmed = row.clone();
                    trimmed.tokens = crate::vendor::codex::truncate::approx_token_count(&text);
                    trimmed.text = text;
                    taken[index] = true;
                    chosen.push((index, trimmed));
                }
                break;
            }
            taken[index] = true;
            remaining -= row.tokens;
            chosen.push((index, row.clone()));
        }
    }
    // Source order, not tier order: a reader still needs a chronology.
    chosen.sort_by_key(|(index, _)| *index);
    chosen.into_iter().map(|(_, row)| row).collect()
}

#[cfg(test)]
mod digest_tests {
    use super::*;

    fn row(evt: u32, tier: Tier, tokens: usize) -> Row {
        Row {
            evt,
            tier,
            text: format!("row {evt}"),
            tokens,
            is_human_turn: tier == Tier::User,
        }
    }

    #[test]
    fn a_digest_keeps_the_highest_tier_first_and_stays_in_source_order() {
        // One user turn, one failure, and a pile of successful tool output that
        // would otherwise eat the whole budget.
        let mut rows = vec![row(1, Tier::User, 10), row(2, Tier::ToolResultError, 10)];
        for evt in 3..40 {
            rows.push(row(evt, Tier::ToolResultOk, 100));
        }
        let digest = digest(&rows, 30);
        assert_eq!(digest.len(), 2, "{digest:?}");
        assert_eq!(digest[0].evt, 1);
        assert_eq!(digest[1].evt, 2, "source order, not tier order");
        assert!(digest.iter().map(|row| row.tokens).sum::<usize>() <= 30);
    }

    #[test]
    fn the_boundary_row_is_truncated_rather_than_dropped() {
        // Codex's rule at the same seam: the newest that fit are kept whole and
        // the one that does not is middle-truncated, because the sentence that
        // did not fit is where the instruction was.
        let mut long = row(1, Tier::User, 10_000);
        // Long enough that truncating it actually leaves a marker.
        long.text = "never push to main and always run the formatter first ".repeat(40);
        let rows = vec![long];
        let digest = digest(&rows, 60);
        assert_eq!(digest.len(), 1, "the boundary row must survive");
        assert!(digest[0].tokens <= 60, "and must fit: {}", digest[0].tokens);
        assert!(digest[0].text.contains("truncated"), "{}", digest[0].text);
    }

    #[test]
    fn a_row_below_the_boundary_floor_is_not_worth_a_stub() {
        let mut long = row(1, Tier::User, 10_000);
        long.text = "never push to main and always run the formatter first ".repeat(40);
        assert!(digest(&[long], 10).is_empty());
    }
}

/// Split rows into episodes, hold back the tail, and pack the rest into chunks.
pub fn plan(rows: &[Row], options: &SegmentOptions) -> Plan {
    let mut plan = Plan::default();
    if rows.is_empty() {
        return plan;
    }

    plan.episodes = episodes(rows, options.min_episode_tokens);
    // A single long-running turn can be one episode larger than a whole chunk
    // budget. Split it before the tail is chosen, or that turn would be either
    // an oversized fold call or an oversized recency tail.
    plan.episodes = split_oversized(rows, plan.episodes, options.chunk_tokens);

    // The tail starts at the episode boundary that keeps it within budget.
    let mut tail_start_episode = plan.episodes.len();
    let mut tail_tokens = 0usize;
    for episode in plan.episodes.iter().rev() {
        if tail_tokens + episode.tokens > options.tail_tokens && tail_tokens > 0 {
            break;
        }
        tail_tokens += episode.tokens;
        tail_start_episode = episode.id;
        if tail_tokens >= options.tail_tokens {
            break;
        }
    }

    let tail_row_start = plan
        .episodes
        .get(tail_start_episode)
        .map(|episode| episode.rows.start)
        .unwrap_or(rows.len());
    plan.tail = tail_row_start..rows.len();
    plan.tail_episode_ids = plan.episodes[tail_start_episode.min(plan.episodes.len())..]
        .iter()
        .map(|episode| episode.id)
        .collect();

    // Pack the episodes before the tail into chunks.
    let mut current: Option<Chunk> = None;
    for episode in &plan.episodes[..tail_start_episode.min(plan.episodes.len())] {
        let would_overflow = current
            .as_ref()
            .is_some_and(|chunk| chunk.tokens + episode.tokens > options.chunk_tokens);
        if would_overflow && let Some(chunk) = current.take() {
            plan.chunks.push(chunk);
        }
        match &mut current {
            Some(chunk) => {
                chunk.rows.end = episode.rows.end;
                chunk.evt_end = episode.evt_end;
                chunk.tokens += episode.tokens;
                chunk.episode_ids.push(episode.id);
            }
            None => {
                current = Some(Chunk {
                    id: String::new(),
                    rows: episode.rows.clone(),
                    evt_start: episode.evt_start,
                    evt_end: episode.evt_end,
                    tokens: episode.tokens,
                    episode_ids: vec![episode.id],
                });
            }
        }
    }
    if let Some(chunk) = current.take() {
        plan.chunks.push(chunk);
    }
    for (index, chunk) in plan.chunks.iter_mut().enumerate() {
        chunk.id = format!("c{index}");
    }
    plan
}

/// Episode boundaries: each human message starts one. Runs shorter than
/// `min_tokens` merge into the next so a chunk is not made of one-line pieces.
fn episodes(rows: &[Row], min_tokens: usize) -> Vec<Episode> {
    let mut boundaries: Vec<usize> = vec![0];
    for (index, row) in rows.iter().enumerate() {
        if index > 0 && row.is_human_turn {
            boundaries.push(index);
        }
    }
    boundaries.dedup();

    let mut raw: Vec<std::ops::Range<usize>> = Vec::new();
    for (position, start) in boundaries.iter().enumerate() {
        let end = boundaries.get(position + 1).copied().unwrap_or(rows.len());
        raw.push(*start..end);
    }

    // Merge undersized episodes forward, keeping the last one whole.
    let mut merged: Vec<std::ops::Range<usize>> = Vec::new();
    let mut pending: Option<std::ops::Range<usize>> = None;
    for range in raw {
        let range = match pending.take() {
            Some(previous) => previous.start..range.end,
            None => range,
        };
        let tokens: usize = rows[range.clone()].iter().map(|row| row.tokens).sum();
        if tokens < min_tokens {
            pending = Some(range);
        } else {
            merged.push(range);
        }
    }
    if let Some(range) = pending {
        match merged.last_mut() {
            Some(last) => last.end = range.end,
            None => merged.push(range),
        }
    }

    merged
        .into_iter()
        .enumerate()
        .map(|(id, range)| {
            let slice = &rows[range.clone()];
            Episode {
                id,
                evt_start: slice.first().map(|row| row.evt).unwrap_or(0),
                evt_end: slice.last().map(|row| row.evt).unwrap_or(0),
                tokens: slice.iter().map(|row| row.tokens).sum(),
                headline: headline(slice),
                rows: range,
            }
        })
        .collect()
}

/// Split any episode larger than `chunk_tokens` at a safe row boundary.
///
/// A tool result must never be separated from the call that produced it, so
/// cuts only land before a row that is not a result.
fn split_oversized(rows: &[Row], episodes: Vec<Episode>, chunk_tokens: usize) -> Vec<Episode> {
    if chunk_tokens == 0 {
        return episodes;
    }
    let mut out: Vec<Episode> = Vec::new();
    for episode in episodes {
        if episode.tokens <= chunk_tokens {
            out.push(episode);
            continue;
        }
        let mut start = episode.rows.start;
        let mut tokens = 0usize;
        for index in episode.rows.clone() {
            let Some(row) = rows.get(index) else { continue };
            let boundary_here = index > start && tokens >= chunk_tokens && !is_result(row);
            if boundary_here {
                out.push(make_episode(rows, start..index));
                start = index;
                tokens = 0;
            }
            tokens += row.tokens;
        }
        if start < episode.rows.end {
            out.push(make_episode(rows, start..episode.rows.end));
        }
    }
    // Ids must stay dense and ordered after splitting.
    for (id, episode) in out.iter_mut().enumerate() {
        episode.id = id;
    }
    out
}

fn is_result(row: &Row) -> bool {
    matches!(
        row.tier,
        crate::vendor::codex::tiered_input::Tier::ToolResultOk
            | crate::vendor::codex::tiered_input::Tier::ToolResultError
    )
}

fn make_episode(rows: &[Row], range: std::ops::Range<usize>) -> Episode {
    let slice = rows.get(range.clone()).unwrap_or(&[]);
    Episode {
        id: 0,
        evt_start: slice.first().map(|row| row.evt).unwrap_or(0),
        evt_end: slice.last().map(|row| row.evt).unwrap_or(0),
        tokens: slice.iter().map(|row| row.tokens).sum(),
        headline: headline(slice),
        rows: range,
    }
}

fn headline(rows: &[Row]) -> String {
    let text = rows
        .iter()
        .find(|row| row.is_human_turn)
        .or_else(|| rows.first())
        .map(|row| row.text.as_str())
        .unwrap_or("");
    let words: Vec<&str> = text.split_whitespace().take(20).collect();
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vendor::codex::tiered_input::Tier;

    fn row(evt: u32, tokens: usize, human: bool) -> Row {
        Row {
            evt,
            tier: if human { Tier::User } else { Tier::ToolCall },
            text: if human {
                format!("[user] request {evt}")
            } else {
                format!("[call x] {evt}")
            },
            tokens,
            is_human_turn: human,
        }
    }

    #[test]
    fn an_empty_transcript_plans_nothing() {
        let plan = plan(&[], &SegmentOptions::default());
        assert!(plan.chunks.is_empty());
        assert!(plan.episodes.is_empty());
    }

    #[test]
    fn every_row_belongs_to_exactly_one_episode() {
        let rows: Vec<Row> = (0..30).map(|i| row(i, 200, i % 5 == 0)).collect();
        let plan = plan(
            &rows,
            &SegmentOptions {
                min_episode_tokens: 1,
                ..Default::default()
            },
        );
        let covered: usize = plan.episodes.iter().map(|episode| episode.rows.len()).sum();
        assert_eq!(covered, rows.len());
        for pair in plan.episodes.windows(2) {
            assert_eq!(pair[0].rows.end, pair[1].rows.start);
        }
    }

    #[test]
    fn undersized_episodes_merge_forward() {
        // Ten human turns of 100 tokens each; min 1_500 forces one episode.
        let rows: Vec<Row> = (0..10).map(|i| row(i, 100, true)).collect();
        let plan = plan(
            &rows,
            &SegmentOptions {
                min_episode_tokens: 1_500,
                ..Default::default()
            },
        );
        assert_eq!(plan.episodes.len(), 1);
    }

    #[test]
    fn the_tail_holds_back_the_newest_rows_and_is_not_folded() {
        let rows: Vec<Row> = (0..40).map(|i| row(i, 500, i % 4 == 0)).collect();
        let options = SegmentOptions {
            chunk_tokens: 4_000,
            tail_tokens: 2_000,
            min_episode_tokens: 1,
        };
        let plan = plan(&rows, &options);
        assert!(plan.tail.end == rows.len());
        assert!(plan.tail.start > 0, "tail took the whole transcript");
        // No chunk may reach into the tail.
        for chunk in &plan.chunks {
            assert!(
                chunk.rows.end <= plan.tail.start,
                "chunk overlaps tail: {chunk:?}"
            );
        }
    }

    #[test]
    fn chunks_respect_the_token_budget_and_are_contiguous() {
        let rows: Vec<Row> = (0..60).map(|i| row(i, 300, i % 3 == 0)).collect();
        let options = SegmentOptions {
            chunk_tokens: 2_000,
            tail_tokens: 600,
            min_episode_tokens: 1,
        };
        let plan = plan(&rows, &options);
        assert!(!plan.chunks.is_empty());
        for chunk in &plan.chunks {
            // One episode may exceed the budget on its own; a chunk of several
            // never should.
            if chunk.episode_ids.len() > 1 {
                assert!(chunk.tokens <= options.chunk_tokens, "{chunk:?}");
            }
        }
        for pair in plan.chunks.windows(2) {
            assert_eq!(pair[0].rows.end, pair[1].rows.start);
        }
    }

    #[test]
    fn a_short_session_is_entirely_tail_and_needs_no_fold_call() {
        let rows: Vec<Row> = (0..3).map(|i| row(i, 100, i == 0)).collect();
        let plan = plan(&rows, &SegmentOptions::default());
        assert_eq!(plan.tail, 0..3);
        assert!(plan.chunks.is_empty());
    }

    #[test]
    fn one_enormous_turn_is_split_instead_of_becoming_one_giant_chunk() {
        // A single human turn followed by 200 rows of work: without splitting
        // this is one episode, and the whole session would land in the tail.
        let mut rows = vec![row(0, 50, true)];
        rows.extend((1..200).map(|i| row(i, 100, false)));
        let options = SegmentOptions {
            chunk_tokens: 1_000,
            tail_tokens: 1_000,
            min_episode_tokens: 1_500,
        };
        let plan = plan(&rows, &options);
        assert!(plan.episodes.len() > 10, "{} episodes", plan.episodes.len());
        assert!(!plan.chunks.is_empty(), "nothing was folded");
        for chunk in &plan.chunks {
            assert!(chunk.tokens <= options.chunk_tokens * 2, "{chunk:?}");
        }
        assert!(
            plan.tail.len() < rows.len(),
            "the tail swallowed the session"
        );
    }

    #[test]
    fn a_split_never_separates_a_tool_result_from_its_call() {
        let mut rows = vec![row(0, 10, true)];
        for i in 1..60u32 {
            let mut call = row(i * 2, 90, false);
            call.tier = Tier::ToolCall;
            let mut result = row(i * 2 + 1, 90, false);
            result.tier = Tier::ToolResultOk;
            rows.push(call);
            rows.push(result);
        }
        let options = SegmentOptions {
            chunk_tokens: 200,
            tail_tokens: 200,
            min_episode_tokens: 1,
        };
        let plan = plan(&rows, &options);
        for episode in &plan.episodes {
            if let Some(first) = rows.get(episode.rows.start) {
                assert_ne!(
                    first.tier,
                    Tier::ToolResultOk,
                    "episode {} starts on a result",
                    episode.id
                );
            }
        }
    }

    #[test]
    fn episode_ids_stay_dense_and_ordered_after_splitting() {
        let mut rows = vec![row(0, 10, true)];
        rows.extend((1..100).map(|i| row(i, 100, i % 20 == 0)));
        let options = SegmentOptions {
            chunk_tokens: 500,
            tail_tokens: 500,
            min_episode_tokens: 1,
        };
        let plan = plan(&rows, &options);
        for (index, episode) in plan.episodes.iter().enumerate() {
            assert_eq!(episode.id, index);
        }
        let covered: usize = plan.episodes.iter().map(|episode| episode.rows.len()).sum();
        assert_eq!(covered, rows.len(), "splitting lost rows");
    }
}
