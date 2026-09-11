//! The handoff benchmark (spec §10.4).
//!
//! # Why this exists
//!
//! Surveying the field for a *proven* compaction algorithm turned up no evidence
//! that any of them were measured on the task sctxx performs. The closest
//! published work — *Handoff Debt* (arXiv:2606.02875), SWE-bench Verified, 181
//! handoff tasks, 724 runs across three successor agents — compares repo-only,
//! raw trace, summary notes and structured notes, and finds "no universal
//! ranking". It has **no arm in which the successor can ask for the part of the
//! transcript it needs**, which is the one thing sctxx does differently.
//!
//! So this measures it, and it is deliberately built to be able to lose:
//!
//! - the questions are derived from the session's canonical events, not from the
//!   artifact, so a question can be asked that the artifact does not answer;
//! - they come in two classes, and the second is the interesting one;
//! - the arms include doing nothing and doing the obvious cheap thing, so a win
//!   over *nothing* is not mistaken for a win;
//! - the retrieval arm is genuinely agentic — it asks for an event range and is
//!   given it — because that arm is the entire hypothesis.
//!
//! What it is not: a SWE-bench run. It does not check whether a successor
//! *resolves an issue*. It checks whether a fresh agent, given each arm's
//! context, can answer specific questions about the session — a proxy, and named
//! as one everywhere it is reported.

use crate::ir::{Event, EventKind, Session};
use crate::pipeline::fold::ops::EvtRange;
use crate::pipeline::ledgers::Ledgers;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Which context a successor agent is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Arm {
    /// Nothing but the question. The floor.
    None,
    /// The near-verbatim recency tail. The obvious cheap answer, and the one the
    /// published evidence actually favours.
    Tail,
    /// The whole artifact: brief, items, tail, retrieval pointers.
    Artifact,
    /// The artifact, plus the ability to ask for an event range and be given the
    /// real events. This is the arm nobody has published.
    Retrieval,
}

impl Arm {
    pub const ALL: [Arm; 4] = [Arm::None, Arm::Tail, Arm::Artifact, Arm::Retrieval];

    pub fn label(self) -> &'static str {
        match self {
            Arm::None => "none",
            Arm::Tail => "tail",
            Arm::Artifact => "artifact",
            Arm::Retrieval => "artifact+retrieval",
        }
    }

    /// Whether the arm may ask for more of the transcript.
    pub fn can_expand(self) -> bool {
        self == Arm::Retrieval
    }
}

/// What a question is testing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Class {
    /// The fact should be in the brief: the goal, the constraints, the shape of
    /// the work, the failures that never got fixed.
    Brief,
    /// The fact is somewhere in the middle of a long session, outside anything a
    /// brief or a recency tail would carry. Only retrieval reaches it.
    Deep,
    /// The fact is in the recency tail specifically. Isolates "the tail already
    /// covers this" from "the artifact adds something".
    Recent,
}

impl Class {
    pub fn label(self) -> &'static str {
        match self {
            Class::Brief => "brief",
            Class::Deep => "deep",
            Class::Recent => "recent",
        }
    }
}

/// One checkable question about a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    pub class: Class,
    /// What the successor is asked.
    pub prompt: String,
    /// The string a correct answer must contain, normalized for comparison.
    pub key: String,
    /// The event the answer comes from, for `expand` and for debugging.
    pub evt: u32,
}

impl Question {
    /// Whether an answer contains the key. Whitespace and case are ignored,
    /// because a correct answer that spells a path differently is correct.
    pub fn answered_by(&self, answer: &str) -> bool {
        let answer = normalize(answer);
        let key = normalize(&self.key);
        !key.is_empty() && answer.contains(&key)
    }
}

/// Normalize for comparison: lowercase, punctuation to spaces, single-spaced.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_alphanumeric() || c == '/' || c == '.' || c == '-' || c == '_' {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.extend(c.to_lowercase());
        } else {
            space = true;
        }
    }
    out
}

/// How many questions to draw from each class by default.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub brief: usize,
    pub deep: usize,
    pub recent: usize,
    /// How many times a successor may ask for more transcript.
    pub expansions: usize,
}

impl Default for Options {
    fn default() -> Self {
        // Small on purpose: every question is a model call per arm, and a
        // benchmark nobody runs because it is expensive proves nothing.
        Self {
            brief: 6,
            deep: 8,
            recent: 4,
            expansions: 3,
        }
    }
}

/// Derive checkable questions from a session.
///
/// Every answer is a string that appears verbatim in the session's own events or
/// ledgers, so the key is checkable by substring and no model is involved in
/// deciding who is right.
pub fn questions(session: &Session, ledgers: &Ledgers, options: &Options) -> Vec<Question> {
    let mut out: Vec<Question> = Vec::new();
    let events: Vec<&Event> = session.active_events().collect();

    // ---- brief: the facts a handoff is supposed to carry -------------------
    if let Some(first) = ledgers.user_messages.first() {
        out.push(Question {
            id: "brief-goal".into(),
            class: Class::Brief,
            prompt: "What did the user originally ask for? Quote a distinctive phrase.".into(),
            key: first
                .text
                .split_whitespace()
                .take(8)
                .collect::<Vec<_>>()
                .join(" "),
            evt: first.evt,
        });
    }
    if let Some(busiest) = ledgers
        .files
        .iter()
        .filter(|file| file.edits > 0)
        .max_by_key(|file| (file.edits, file.path.len()))
    {
        out.push(Question {
            id: "brief-busiest-file".into(),
            class: Class::Brief,
            prompt: "Which file did the session edit the most times, and how many times?".into(),
            key: busiest.path.clone(),
            evt: busiest.last_evt,
        });
    }
    if let Some(commit) = ledgers.git.commits.last() {
        out.push(Question {
            id: "brief-last-commit".into(),
            class: Class::Brief,
            prompt: "What was the newest commit made during the session?".into(),
            key: commit.subject.clone(),
            evt: commit.evt,
        });
    }
    if let Some(error) = ledgers.unresolved_errors().first() {
        out.push(Question {
            id: "brief-unresolved".into(),
            class: Class::Brief,
            prompt: "Name an error the session never resolved.".into(),
            key: error.sig.clone(),
            evt: error.last_evt,
        });
    }
    out.push(Question {
        id: "brief-user-turns".into(),
        class: Class::Brief,
        prompt: "How many times did the user speak in this session?".into(),
        key: session.user_turns().to_string(),
        evt: 0,
    });
    out.push(Question {
        id: "brief-dir".into(),
        class: Class::Brief,
        prompt: "Which directory was the work happening in?".into(),
        key: session
            .meta
            .cwd
            .as_ref()
            .map(|cwd| cwd.to_string_lossy().to_string())
            .unwrap_or_default(),
        evt: 0,
    });

    // ---- deep and recent: sampled from events that can actually be asked about
    //
    // Sampling positions blindly and *then* filtering produced five questions
    // where seventeen were asked for, because most events are tool results whose
    // output has no distinctive token. The filter comes first and the spread
    // second, so the requested count is the count that is asked.
    let answerable: Vec<&Event> = events
        .iter()
        .copied()
        .filter(|event| key_for(event).is_some())
        .collect();
    let usable = (answerable.len() * 85) / 100;
    if usable >= options.deep {
        let stride = usable / options.deep.max(1);
        for n in 0..options.deep {
            let index = n * stride + stride / 2;
            if let Some(event) = answerable.get(index)
                && let Some(question) = ask(Class::Deep, n, event, false)
            {
                out.push(question);
            }
        }
    }
    for (n, event) in answerable.iter().rev().take(options.recent).enumerate() {
        if let Some(question) = ask(Class::Recent, n, event, true) {
            out.push(question);
        }
    }

    // Trim to the requested counts, deterministically.
    truncate_class(&mut out, Class::Brief, options.brief);
    truncate_class(&mut out, Class::Deep, options.deep);
    truncate_class(&mut out, Class::Recent, options.recent);
    out
}

/// The most distinctive single token in a blob of text.
///
/// A key has to be something a correct answer would *contain*. Whole phrases do
/// not survive paraphrase — the first run asked "what did the tool call do?" and
/// keyed on the tool name followed by its argument, and a correct answer that
/// described the call scored zero. A path or an identifier is quoted when it is
/// known and absent when it is not, which is the discrimination a handoff
/// benchmark wants.
fn distinctive(text: &str) -> Option<String> {
    let mut best: Option<(u8, std::cmp::Reverse<usize>, String)> = None;
    for raw in text.split_whitespace() {
        let token = raw.trim_matches(|c: char| {
            !(c.is_alphanumeric() || c == '/' || c == '.' || c == '-' || c == '_')
        });
        if token.len() < 8 {
            continue;
        }
        let letters = token.chars().filter(|c| c.is_alphabetic()).count();
        let rank = if token.contains('/') && token.contains('.') {
            0
        } else if token.contains('/') {
            1
        } else if token.contains('_') || (token.chars().any(|c| c.is_uppercase()) && letters > 4) {
            2
        } else if letters >= 6 {
            3
        } else {
            continue;
        };
        let candidate = (rank, std::cmp::Reverse(token.len()), token.to_string());
        let better = best
            .as_ref()
            .is_none_or(|current| (candidate.0, candidate.1) < (current.0, current.1));
        if better {
            best = Some(candidate);
        }
    }
    best.map(|(_, _, token)| token)
}

/// The distinctive string a question about this event would be keyed on, if the
/// event is worth asking about at all.
fn key_for(event: &Event) -> Option<(String, String)> {
    let (prompt, key) = match &event.kind {
        EventKind::UserMessage { text, is_meta } if !is_meta => (
            "What did the user ask for at this point in the session? Quote a distinctive phrase."
                .to_string(),
            text.split_whitespace()
                .take(8)
                .collect::<Vec<_>>()
                .join(" "),
        ),
        EventKind::ToolCall { name, args, .. } => (
            format!(
                "Which tool was called at this point, and what was it given? Name the tool \
                 ({name}) or quote its argument."
            ),
            // The argument, not "name + argument": keying on both scored zero on
            // the first real run because a correct answer described the call
            // instead of reciting it.
            distinctive(&summarize_args(args)).unwrap_or_default(),
        ),
        EventKind::ToolResult { output, .. } => (
            "What did the tool return at this point? Quote a distinctive path or identifier from \
             the result."
                .to_string(),
            distinctive(output).unwrap_or_default(),
        ),
        EventKind::AssistantText { text, .. } => (
            "What did the agent say at this point in the session? Quote a distinctive phrase."
                .to_string(),
            distinctive(text.as_str()).unwrap_or_else(|| {
                text.split_whitespace()
                    .take(8)
                    .collect::<Vec<_>>()
                    .join(" ")
            }),
        ),
        _ => return None,
    };
    // A key shorter than this is matched by accident.
    (normalize(&key).len() >= 6).then_some((prompt, key))
}

/// Turn an event into a question of the given class.
fn ask(class: Class, index: usize, event: &Event, most_recent: bool) -> Option<Question> {
    let (mut prompt, key) = key_for(event)?;
    if most_recent {
        prompt = prompt
            .replace("at this point in the session", "most recently")
            .replace("at this point", "most recently");
    }
    Some(Question {
        id: format!("{}-{index}", class.label()),
        class,
        prompt: format!("At event {}: {prompt}", event.idx),
        key,
        evt: event.idx,
    })
}

fn summarize_args(args: &serde_json::Value) -> String {
    match args {
        serde_json::Value::Object(map) => map
            .values()
            .filter_map(|value| value.as_str())
            .map(|value| {
                let value = value.lines().next().unwrap_or("");
                value.chars().take(60).collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(" "),
        serde_json::Value::String(text) => text.chars().take(60).collect(),
        other => other.to_string(),
    }
}

fn truncate_class(questions: &mut Vec<Question>, class: Class, keep: usize) {
    let mut seen = 0usize;
    questions.retain(|question| {
        if question.class != class {
            return true;
        }
        seen += 1;
        seen <= keep
    });
}

/// One question's outcome under one arm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trial {
    pub question: String,
    pub class: Class,
    pub arm: Arm,
    pub correct: bool,
    /// Prompt tokens the arm sent, summed over any expansion rounds.
    pub tokens: usize,
    /// How many times the successor asked for more transcript.
    pub expansions: usize,
    /// What it actually said. Kept because a benchmark whose failures cannot be
    /// read is a benchmark whose failures cannot be diagnosed — the first run of
    /// this scored 0 % on every deep question under every arm, and the only way
    /// to find out whether that was the harness or the model was to look.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub answer: String,
}

/// What one arm scored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArmScore {
    pub asked: usize,
    pub correct: usize,
    pub tokens: usize,
    pub expansions: usize,
    /// Correct per class.
    pub by_class: BTreeMap<String, (usize, usize)>,
}

impl ArmScore {
    pub fn accuracy(&self) -> f64 {
        if self.asked == 0 {
            0.0
        } else {
            self.correct as f64 / self.asked as f64
        }
    }

    /// Tokens per correct answer. The number that decides whether an arm is
    /// worth running: an arm that is 5 points better at 4× the cost is a
    /// different proposition from one that is 5 points better for free.
    pub fn tokens_per_correct(&self) -> Option<usize> {
        (self.correct > 0).then(|| self.tokens / self.correct)
    }

    pub fn class_accuracy(&self, class: Class) -> Option<f64> {
        self.by_class
            .get(class.label())
            .filter(|(asked, _)| *asked > 0)
            .map(|(asked, correct)| *correct as f64 / *asked as f64)
    }
}

/// Fold trials into per-arm scores.
pub fn score(trials: &[Trial]) -> BTreeMap<Arm, ArmScore> {
    let mut scores: BTreeMap<Arm, ArmScore> = BTreeMap::new();
    for trial in trials {
        let entry = scores.entry(trial.arm).or_default();
        entry.asked += 1;
        entry.tokens += trial.tokens;
        entry.expansions += trial.expansions;
        if trial.correct {
            entry.correct += 1;
        }
        let slot = entry
            .by_class
            .entry(trial.class.label().to_string())
            .or_insert((0, 0));
        slot.0 += 1;
        if trial.correct {
            slot.1 += 1;
        }
    }
    scores
}

/// The range a successor asked for, parsed out of its reply.
///
/// A successor requests more transcript by emitting a line of the form
/// `EXPAND 4122..4381`. Anything else is treated as its answer.
pub fn requested_range(reply: &str) -> Option<EvtRange> {
    for line in reply.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("EXPAND") else {
            continue;
        };
        let rest = rest.trim();
        let (start, end) = rest.split_once("..")?;
        let start = start.trim().trim_end_matches('=').trim().parse().ok()?;
        let end = end
            .trim()
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()?;
        return Some(EvtRange::new(start, end));
    }
    None
}

/// Render the raw events in a range, for the retrieval arm.
pub fn expand(session: &Session, range: EvtRange) -> String {
    let mut out = String::new();
    for event in session.active_events() {
        if !range.contains(event.idx) {
            continue;
        }
        if let Some(line) = describe(event) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    if out.is_empty() {
        out.push_str("(no events in that range)\n");
    }
    out
}

/// One event as a single line, for retrieval.
fn describe(event: &Event) -> Option<String> {
    let body = match &event.kind {
        EventKind::UserMessage { text, is_meta } => {
            format!(
                "[{}] {text}",
                if *is_meta { "user-harness" } else { "user" }
            )
        }
        EventKind::UserAnswer { question, answer } => format!("[user-answer] {question} {answer}"),
        EventKind::AssistantText { text, .. } => format!("[assistant] {text}"),
        EventKind::Reasoning { .. } => return None,
        EventKind::ToolCall { name, args, .. } => {
            format!("[call {name}] {}", summarize_args(args))
        }
        EventKind::ToolResult { output, .. } => {
            format!(
                "[result] {}",
                output.lines().take(20).collect::<Vec<_>>().join("\n")
            )
        }
        _ => return None,
    };
    Some(format!(
        "evt {} {}",
        event.idx,
        body.chars().take(1_200).collect::<String>()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_requested_range_is_read_and_anything_else_is_an_answer() {
        assert_eq!(
            requested_range("EXPAND 4122..4381"),
            Some(EvtRange::new(4122, 4381))
        );
        assert_eq!(
            requested_range("thinking...\nEXPAND 10..20\n"),
            Some(EvtRange::new(10, 20))
        );
        assert_eq!(requested_range("The answer is 42"), None);
        assert_eq!(requested_range("EXPAND nonsense"), None);
    }

    #[test]
    fn an_answer_is_checked_by_substring_ignoring_case_and_punctuation() {
        let question = Question {
            id: "q".into(),
            class: Class::Brief,
            prompt: "which file".into(),
            key: "acryl-tui/src/render.rs".into(),
            evt: 1,
        };
        assert!(question.answered_by("It was `Acryl-TUI/src/render.rs`, I think."));
        assert!(!question.answered_by("acryl-desktop/src/main.ts"));
        assert!(!question.answered_by(""));
    }

    #[test]
    fn a_key_prefers_a_path_over_prose() {
        assert_eq!(
            distinctive("wrote /Users/x/acryl-tui/src/render.rs and some other words here"),
            Some("/Users/x/acryl-tui/src/render.rs".to_string())
        );
        assert_eq!(
            distinctive("the symbol ModelProfileOverlayState was not found"),
            Some("ModelProfileOverlayState".to_string())
        );
        // Prose has no key, so no question is asked from it.
        assert_eq!(distinctive("ok that works now"), None);
    }

    #[test]
    fn scores_count_per_arm_and_per_class() {
        let trials = vec![
            Trial {
                question: "a".into(),
                class: Class::Deep,
                arm: Arm::None,
                correct: false,
                tokens: 10,
                expansions: 0,
                answer: String::new(),
            },
            Trial {
                question: "a".into(),
                class: Class::Deep,
                arm: Arm::Retrieval,
                correct: true,
                tokens: 100,
                expansions: 1,
                answer: String::new(),
            },
        ];
        let scores = score(&trials);
        assert_eq!(scores[&Arm::None].accuracy(), 0.0);
        assert_eq!(scores[&Arm::Retrieval].accuracy(), 1.0);
        assert_eq!(scores[&Arm::Retrieval].expansions, 1);
        assert_eq!(
            scores[&Arm::Retrieval].class_accuracy(Class::Deep),
            Some(1.0)
        );
        assert_eq!(scores[&Arm::Retrieval].tokens_per_correct(), Some(100));
        // An arm with no correct answers has no cost per correct answer, rather
        // than an infinite one that would sort oddly in a report.
        assert_eq!(scores[&Arm::None].tokens_per_correct(), None);
    }

    #[test]
    fn deep_and_recent_are_drawn_from_events_that_can_be_asked_about() {
        // Most events are tool results with no distinctive token. Sampling
        // positions first and filtering afterwards produced five questions where
        // seventeen were asked for, so the filter comes first here.
        let mut events: Vec<Event> = Vec::new();
        for index in 0..100 {
            events.push(Event {
                idx: index,
                native_id: None,
                parent: None,
                ts: None,
                stream: 0,
                kind: if index % 2 == 0 {
                    EventKind::ToolResult {
                        call_id: format!("c{index}"),
                        output: "ok".to_string(),
                        is_error: None,
                        exit_code: None,
                    }
                } else {
                    EventKind::ToolResult {
                        call_id: format!("c{index}"),
                        output: format!("wrote /repo/src/module_{index}/render.rs for the build"),
                        is_error: None,
                        exit_code: None,
                    }
                },
                line: crate::ir::LineRef { path: 0, line: 0 },
            });
        }
        let askable: Vec<&Event> = events
            .iter()
            .filter(|event| key_for(event).is_some())
            .collect();
        assert_eq!(askable.len(), 50, "only the event with a path is askable");
        // The spread stays inside the first 85 %, so a deep question cannot be
        // answered from a recency tail.
        let usable = (askable.len() * 85) / 100;
        assert!(askable[usable - 1].idx < 85);
    }
}
