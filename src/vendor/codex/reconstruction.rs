// Portions derived from OpenAI Codex (https://github.com/openai/codex),
// commit 818f1cca8ccf8899f0f4d59336baebaccf358eed, file
// codex-rs/core/src/session/rollout_reconstruction.rs.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0.
// Modified by the sctxx authors: kept only the rollback semantics (a
// `ThreadRolledBack { num_turns }` event drops the newest surviving user-turn
// segments) and reduced the output to the surviving event indices. None of the
// Codex-internal history, world-state, or window bookkeeping is reproduced.

//! Rollback-aware replay of a linear event stream.
//!
//! Codex records an undo as an event rather than by rewriting the rollout, so a
//! naive reader compacts work the user already threw away. Replay splits the
//! stream into user-turn segments and drops the newest `num_turns` of them for
//! every rollback, walking backwards so consecutive rollbacks compose.

/// One event as far as replay is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReplayEvent {
    /// Starts a new user turn (a real human message).
    UserTurnBoundary,
    /// Drops the newest `num_turns` surviving user turns.
    Rollback { num_turns: u32 },
    /// Anything else; belongs to the turn that is currently open.
    Other,
}

/// Return the indices that survive every rollback, in chronological order.
pub fn surviving_indices(events: &[ReplayEvent]) -> Vec<u32> {
    // Split into segments: each starts at a user-turn boundary. Events before
    // the first boundary (session metadata, harness context) form segment 0,
    // which is not a user turn and is therefore never rolled back.
    struct Segment {
        indices: Vec<u32>,
        is_user_turn: bool,
    }
    let mut segments: Vec<Segment> = vec![Segment {
        indices: Vec::new(),
        is_user_turn: false,
    }];
    let mut rollbacks: Vec<(usize, u32)> = Vec::new();

    for (idx, event) in events.iter().enumerate() {
        let idx = idx as u32;
        match event {
            ReplayEvent::UserTurnBoundary => {
                segments.push(Segment {
                    indices: vec![idx],
                    is_user_turn: true,
                });
            }
            ReplayEvent::Rollback { num_turns } => {
                // The rollback event itself is not part of the transcript.
                rollbacks.push((segments.len() - 1, *num_turns));
            }
            ReplayEvent::Other => {
                if let Some(last) = segments.last_mut() {
                    last.indices.push(idx);
                }
            }
        }
    }

    let mut dropped = vec![false; segments.len()];
    // Apply rollbacks newest-first so a rollback of a rollback composes.
    for (at_segment, num_turns) in rollbacks.iter().rev() {
        let mut remaining = *num_turns;
        let mut cursor = *at_segment;
        while remaining > 0 {
            if !dropped[cursor] && segments[cursor].is_user_turn {
                dropped[cursor] = true;
                remaining -= 1;
            }
            if cursor == 0 {
                break;
            }
            cursor -= 1;
        }
    }

    let mut out = Vec::new();
    for (segment, dropped) in segments.iter().zip(dropped) {
        if !dropped {
            out.extend_from_slice(&segment.indices);
        }
    }
    out.sort_unstable();
    out
}

#[cfg(test)]
mod tests {
    use super::ReplayEvent::*;
    use super::*;

    #[test]
    fn a_stream_without_rollbacks_survives_whole() {
        let events = [Other, UserTurnBoundary, Other, UserTurnBoundary, Other];
        assert_eq!(surviving_indices(&events), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn one_rollback_drops_the_last_user_turn() {
        // evt 0 user, 1 work, 2 user, 3 work, 4 rollback(1)
        let events = [
            UserTurnBoundary,
            Other,
            UserTurnBoundary,
            Other,
            Rollback { num_turns: 1 },
        ];
        assert_eq!(surviving_indices(&events), vec![0, 1]);
    }

    #[test]
    fn a_rollback_of_two_turns_drops_both() {
        let events = [
            UserTurnBoundary,
            UserTurnBoundary,
            Other,
            UserTurnBoundary,
            Other,
            Rollback { num_turns: 2 },
        ];
        assert_eq!(surviving_indices(&events), vec![0]);
    }

    #[test]
    fn work_after_a_rollback_survives() {
        let events = [
            UserTurnBoundary,
            UserTurnBoundary,
            Other,
            Rollback { num_turns: 1 },
            UserTurnBoundary,
            Other,
        ];
        assert_eq!(surviving_indices(&events), vec![0, 4, 5]);
    }

    #[test]
    fn consecutive_rollbacks_compose() {
        let events = [
            UserTurnBoundary,
            UserTurnBoundary,
            UserTurnBoundary,
            Rollback { num_turns: 1 },
            Rollback { num_turns: 1 },
        ];
        assert_eq!(surviving_indices(&events), vec![0]);
    }

    #[test]
    fn a_rollback_larger_than_the_history_empties_it_without_panicking() {
        let events = [UserTurnBoundary, Other, Rollback { num_turns: 99 }];
        assert!(surviving_indices(&events).is_empty());
    }

    #[test]
    fn events_before_the_first_user_turn_are_never_rolled_back() {
        let events = [Other, UserTurnBoundary, Other, Rollback { num_turns: 5 }];
        assert_eq!(surviving_indices(&events), vec![0]);
    }
}
