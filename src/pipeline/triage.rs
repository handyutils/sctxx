//! S1b — deterministic typed extraction (Knowledge Triage).
//!
//! Source: Zerhoudi, Mitrović, Granitzer, *The Compaction Cliff in Long-Running
//! AI Agent Memory*, arXiv:2608.22752 (CIKM 2026), §3. The reading is in
//! `docs/research/2026-09-11-paper-knowledge-triage-typecompact.md`; the
//! decision is ADR 0008.
//!
//! # Why this module exists
//!
//! `ItemKind::Constraint` is first in [`crate::pipeline::fold::ops::ItemKind::PRIORITY`],
//! the artifact renders it under **Hard constraints**, and the artifact's own
//! preamble instructs its reader to treat that section as binding. But the only
//! producer of a `Constraint` item was the fold, and ADR 0007 made the fold
//! opt-in. With the default `--llm none` the section was therefore *always*
//! empty, while the artifact went on telling its reader to obey it.
//!
//! The paper's central measurement is the cost of that arrangement in a
//! different setting: type-blind compaction retains 0.53 of an agent's safety
//! rules at a 50 % budget and 0.10 after five rounds, because *a type-blind
//! compactor has no signal for which sentences are safety rules*. Its
//! recommendation for no-LLM deployments is an explicit classifier, which is
//! what this is.
//!
//! # What is deliberately not generalised from the paper
//!
//! The paper classifies authored configuration artifacts — rules files, prompts,
//! instruction documents — where a bare imperative ("Run the tests before
//! pushing") is a procedure and the imperative mood is the dominant form. This
//! module classifies *turns inside a transcript*, where the same sentence is a
//! **task**, not a standing rule. The signals therefore differ: this module looks
//! for deontic markers that stand outside the work (`never`, `must not`,
//! `always`, `do not … without …`) and deliberately ignores the imperative mood,
//! which in a session is what the user is asking for right now.
//!
//! # Recall, stated plainly
//!
//! A rule stated declaratively — "the schema is frozen until the migration
//! lands" — carries no marker and is invisible here. The paper measures exactly
//! this: declarative phrasing is 49.8 % of real safety text, and a regex
//! classifier's recall on it is **0**. This module is a floor, not a solution;
//! every artifact it produces says so, and
//! [`Triage::recall_note`] is the sentence it says it in.

use crate::ir::{EventIdx, EventKind, Session};
use crate::pipeline::fold::ops::{Confidence, EvtRange, ItemKind, NewItem};
use crate::pipeline::fold::state::FoldState;
use crate::pipeline::ledgers::Ledgers;
use crate::vendor::codex::secrets::{RedactMode, redact};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How many constraints an artifact carries.
///
/// A handoff with 200 "binding" lines has no binding lines. The cap is applied
/// after ranking, and what it dropped is always reported.
pub const MAX_CONSTRAINTS: usize = 40;

/// Longest sentence worth calling a constraint. Past this it is prose, a paste,
/// or a log line, and quoting it verbatim costs more than it says.
const MAX_SENTENCE_CHARS: usize = 240;

/// Shortest. "Never." is not actionable without its object.
const MIN_SENTENCE_WORDS: usize = 3;

/// How a standing instruction is recognised.
///
/// Every marker here is anchored at the **head of the clause**, and that is the
/// whole design. A rule is stated in the imperative, so its directive comes
/// first; the same words mid-sentence are a description. The first run of this
/// classifier did not anchor them, and on a real session it returned "CONTRACTS
/// YOUR HOT-RELOAD / LIFECYCLE WORK MUST NOT BREAK" and "intercept() and
/// fetch.register() don't touch webServer" as binding user instructions. Both
/// contain a marker and neither is an instruction: the first is a heading, the
/// second says what some code does not do.
///
/// So: `never push to main` is a rule; `X must not break` is a property;
/// `the tests must stay green` is a property that *reads* like a rule and is
/// therefore a known miss, not a false positive. Retrieving those without a
/// model needs more than a pattern, and the artifact says so where it matters.
const DIRECTIVES: &[(&str, &[&str])] = &[
    ("prohibition", &["never ", "never,", "never ever"]),
    (
        "negative directive",
        &[
            "do not ",
            "don't ",
            "dont ",
            "you must not ",
            "you must never ",
            "we must not ",
            "you should not ",
            "you shouldn't ",
            "you should never ",
            "must not ",
            "i need you to never ",
        ],
    ),
    (
        "standing directive",
        &[
            "always ",
            "at all times",
            "every time you",
            "you should always ",
            "you must always ",
            "i need you to always ",
        ],
    ),
    // There is deliberately no `make sure` / `ensure` / `be sure` family here,
    // and no bare `you must`. In an authored rules file those open a rule; in a
    // transcript they open a *task* — "make sure we don't have a floating config"
    // is work to do now, not a rule for whoever comes next. Measured on a real
    // 274-turn session, that family contributed two of the three hits and both
    // were tasks. The rule-reading of a sentence and the task-reading of it are
    // separated by what a model knows about the world, not by a pattern.
    (
        "untouchable",
        &[
            "don't touch",
            "do not touch",
            "leave the ",
            "leave it alone",
            "hands off",
            "don't change",
            "do not change",
            "don't modify",
            "do not modify",
            "don't rename",
            "do not rename",
        ],
    ),
    (
        "reminder",
        &[
            "don't forget",
            "do not forget",
            "never forget",
            "keep in mind that",
            "bear in mind that",
        ],
    ),
    (
        "permission gate",
        &[
            "ask before",
            "ask me before",
            "check with me",
            "without my permission",
            "without permission",
            "needs my approval",
        ],
    ),
    ("emphasis", &["critical:", "important:", "crucial:"]),
    ("exclusivity", &["only ever ", "only use ", "stick to "]),
];

/// Words a speaker puts in front of a rule without changing it. They are skipped
/// before the head test, so "and also, never commit secrets" is still a rule.
const LEADING: &[&str] = &[
    "please",
    "also",
    "and",
    "but",
    "then",
    "so",
    "plus",
    "finally",
    "lastly",
    "importantly",
    "however",
    "just",
    "note",
    "ps",
    "p.s.",
    "one more thing",
];

/// Openers that make a sentence a question about the work, whatever it contains.
const QUESTION_OPENERS: &[&str] = &[
    "why ",
    "how ",
    "what ",
    "when ",
    "where ",
    "which ",
    "who ",
    "can you",
    "could you",
    "would you",
    "is it",
    "are we",
    "did you",
    "do you",
    "does ",
];

/// Text that is harness noise rather than a human turn, even when the adapter
/// did not mark it as meta.
const NOISE: &[&str] = &[
    "<command-name>",
    "<command-message>",
    "<local-command",
    "system-reminder",
    "<system-reminder",
    "[request interrupted",
    "caveat: the messages below",
];

/// Directory names that say nothing about what a constraint governs.
const GENERIC_DIRS: &[&str] = &[
    "src",
    "lib",
    "dist",
    "build",
    "out",
    "target",
    "node_modules",
    "vendor",
    "bin",
    "obj",
    "tmp",
    "temp",
    "test",
    "tests",
    "__tests__",
    "spec",
    "specs",
    "fixtures",
    "fixture",
    "examples",
    "example",
    "assets",
    "static",
    "public",
    "coverage",
    "generated",
    "gen",
    "pkg",
    "packages",
    "apps",
    "app",
    "crates",
    "modules",
    "components",
    "shared",
    "common",
    "internal",
    "server",
    "client",
    "types",
    "utils",
    "helpers",
    "hooks",
    "styles",
    "images",
    "img",
    "docs",
    "doc",
];

/// One deterministic constraint: a standing instruction the user gave, with the
/// subsystems it governs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Constraint {
    /// The user's own words, one line, trimmed. Rendered verbatim.
    pub text: String,
    /// The event the sentence came from — one event, because it is one sentence.
    pub evt: EventIdx,
    /// Which marker families matched, in [`CONSTRAINTS`] order.
    pub markers: Vec<String>,
    /// Subsystems this governs. Empty means **global**: it applies everywhere,
    /// so it is replicated into every partition. That is the safe default — an
    /// imperfect scope over-replicates, it never under-replicates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope: Vec<String>,
}

impl Constraint {
    /// How strongly this reads as a standing instruction. Used for ranking only.
    pub fn weight(&self) -> usize {
        self.markers.len()
    }

    /// The form the verifier looks for: lowercased, punctuation-free, single
    /// spaced. Zero-distortion preservation means this string survives intact.
    pub fn normalized(&self) -> String {
        normalize(&self.text)
    }
}

/// What the deterministic pass found, and what it could not.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Triage {
    pub constraints: Vec<Constraint>,
    /// Non-meta user messages scanned.
    pub messages: usize,
    /// Candidate sentences considered.
    pub sentences: usize,
    /// Sentences that matched more than one constraint and were folded into one.
    pub duplicates: usize,
    /// Constraints found beyond [`MAX_CONSTRAINTS`].
    pub dropped: usize,
}

impl Triage {
    pub fn is_empty(&self) -> bool {
        self.constraints.is_empty()
    }

    /// The honest statement of what this layer cannot see. Rendered into the
    /// artifact whenever the semantic layer did not run, because a reader who
    /// does not know the rule for what it is will over-trust the section.
    pub fn recall_note(&self) -> String {
        let found = self.constraints.len();
        format!(
            "found by pattern, not by model: {found} standing instruction(s) across {} user message(s). \
             A rule stated declaratively — \"the schema is frozen until the migration lands\" — carries no \
             marker and is not here; measured recall for this kind of classifier on declarative text is 0. \
             Treat the absence of a rule as unknown, not as permission.",
            self.messages
        )
    }
}

/// One constraint's fate between the triage pass and the rendered artifact.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GuardReport {
    /// Constraints the triage pass found.
    pub found: usize,
    /// Still present, verbatim, in the final active state.
    pub preserved: usize,
    /// Dropped by the semantic layer and put back by the verifier.
    pub restored: usize,
    /// Dropped and not recoverable. Non-zero is a defect worth a non-zero exit.
    pub missing: Vec<String>,
}

impl GuardReport {
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty()
    }
}

/// Deterministic typed extraction over the session's non-meta user turns.
pub fn run(session: &Session, ledgers: &Ledgers, mode: RedactMode) -> Triage {
    let vocabulary = subsystem_vocabulary(ledgers);
    let mut triage = Triage::default();
    // Keyed by the normalized form so a rule restated in three ways is one rule.
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();

    for event in session.active_events() {
        let EventKind::UserMessage {
            text,
            is_meta: false,
        } = &event.kind
        else {
            continue;
        };
        triage.messages += 1;
        let text = redact(text, mode);

        for sentence in sentences(&text) {
            triage.sentences += 1;
            let Some(constraint) = classify(&sentence, event.idx, &vocabulary) else {
                continue;
            };
            let key = constraint.normalized();
            if key.is_empty() {
                continue;
            }
            if let Some(existing) = seen.get(&key).copied() {
                // Same rule, said again: keep the earlier provenance (the first
                // statement is the one the rest of the session was working
                // under) and count the restatement.
                triage.duplicates += 1;
                let kept = &mut triage.constraints[existing];
                if kept.markers.len() < constraint.markers.len() {
                    kept.markers = constraint.markers;
                }
                continue;
            }
            // A sentence that contains another kept constraint adds nothing.
            if let Some((index, _)) = seen
                .iter()
                .find(|(other, _)| other.len() > key.len() && other.contains(&key))
                .map(|(other, index)| (other.clone(), *index))
            {
                triage.duplicates += 1;
                let _ = index;
                continue;
            }
            seen.insert(key, triage.constraints.len());
            triage.constraints.push(constraint);
        }
    }

    rank(&mut triage);
    triage
}

/// Strongest first, then most recent — a rule restated late was restated for a
/// reason. The tie-break is the event index, so the order is total and stable.
fn rank(triage: &mut Triage) {
    triage.constraints.sort_by(|a, b| {
        b.weight()
            .cmp(&a.weight())
            .then_with(|| b.evt.cmp(&a.evt))
            .then_with(|| a.text.cmp(&b.text))
    });
    if triage.constraints.len() > MAX_CONSTRAINTS {
        triage.dropped = triage.constraints.len() - MAX_CONSTRAINTS;
        triage.constraints.truncate(MAX_CONSTRAINTS);
    }
}

/// Classify one sentence, or reject it.
///
/// Rejection is cheap and conservative on purpose: this section is presented to
/// its reader as *binding*, so a false positive costs more than a miss.
fn classify(sentence: &str, evt: EventIdx, vocabulary: &[String]) -> Option<Constraint> {
    let text = clean(sentence);
    if text.chars().count() > MAX_SENTENCE_CHARS {
        return None;
    }
    if text.split_whitespace().count() < MIN_SENTENCE_WORDS {
        return None;
    }
    let lower = text.to_lowercase();
    if NOISE.iter().any(|noise| lower.contains(noise)) {
        return None;
    }
    if lower.trim_end().ends_with('?') || lower.trim_end().ends_with(':') {
        return None;
    }
    if QUESTION_OPENERS
        .iter()
        .any(|opener| lower.starts_with(opener))
    {
        return None;
    }
    // A heading in capitals is a title, not an instruction to anyone.
    if is_shouted(&text) {
        return None;
    }
    // A constraint on the *artifact's* reader is not a constraint on this work.
    let head = directive_head(&lower);
    let markers: Vec<String> = DIRECTIVES
        .iter()
        .filter(|(_, needles)| needles.iter().any(|needle| head.starts_with(needle)))
        .map(|(family, _)| (*family).to_string())
        .collect();
    if markers.is_empty() {
        return None;
    }
    Some(Constraint {
        scope: scope_of(&lower, vocabulary),
        text,
        evt,
        markers,
    })
}

/// The clause with the pleasantries removed, so the directive can be tested at
/// its head. `from now on` is folded in because it is a scope on the rule rather
/// than part of it.
fn directive_head(lower: &str) -> &str {
    let mut head = lower.trim_start();
    loop {
        let before = head;
        for filler in LEADING {
            if let Some(rest) = head.strip_prefix(filler)
                && !rest.chars().next().is_some_and(char::is_alphanumeric)
            {
                head = rest
                    .trim_start_matches(['.', ',', ':', ';', ' ', '-'])
                    .trim_start();
                break;
            }
        }
        if let Some(rest) = head.strip_prefix("from now on") {
            head = rest
                .trim_start_matches(['.', ',', ':', ' ', '-'])
                .trim_start();
        }
        if head == before {
            break;
        }
    }
    head
}

/// True when every word of two or more letters is capitalised — a heading
/// someone pasted, not something they said to the agent.
fn is_shouted(text: &str) -> bool {
    let words: Vec<&str> = text
        .split_whitespace()
        .filter(|word| word.chars().filter(|c| c.is_alphabetic()).count() >= 2)
        .collect();
    words.len() >= 4
        && words.iter().all(|word| {
            let mut letters = word.chars().filter(|c| c.is_alphabetic());
            letters.all(|c| !c.is_lowercase())
        })
}

/// `needle` matched case-insensitively. Needles that are a single short word
/// must also sit on a word boundary, so "never" does not match "whatever".
fn contains_word(haystack: &str, needle: &str) -> bool {
    let needle = needle.trim_end();
    let mut from = 0;
    while let Some(at) = haystack[from..].find(needle) {
        let start = from + at;
        let end = start + needle.len();
        let before_ok = start == 0
            || !haystack[..start]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric());
        let after_ok = end >= haystack.len()
            || !haystack[end..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
        if from >= haystack.len() {
            break;
        }
    }
    false
}

/// Directory names that appear in at least two ledger files, longest first.
///
/// This is the paper's `π : I → leaves(T)` in the only form a session offers:
/// the project's own directory names, taken from what the session actually
/// touched rather than from a configured topic tree.
fn subsystem_vocabulary(ledgers: &Ledgers) -> Vec<String> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for file in &ledgers.files {
        for component in file.path.split('/') {
            if component.len() < 3
                || GENERIC_DIRS.contains(&component)
                || component.starts_with('.')
                || component.contains('.')
            {
                continue;
            }
            *counts.entry(component).or_default() += 1;
        }
    }
    let mut vocabulary: Vec<String> = counts
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .map(|(component, _)| component.to_string())
        .collect();
    // Longest first so "harness-runtime" is tested before "harness".
    vocabulary.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    vocabulary
}

/// Which subsystems a constraint names. Empty means global.
///
/// The paper's `σ : I_C → 2^{leaves(T)}`, restricted to what is decidable: a
/// constraint governs the subsystems it names. Naming none is read as governing
/// everything, which is the conservative direction — over-replication costs
/// tokens, under-replication loses the rule.
fn scope_of(lower: &str, vocabulary: &[String]) -> Vec<String> {
    let mut scope: Vec<String> = Vec::new();
    for name in vocabulary {
        if contains_word(lower, &name.to_lowercase()) {
            scope.push(name.clone());
        }
    }
    scope.sort();
    scope.dedup();
    scope
}

/// Split a user message into candidate sentences.
///
/// Fenced code blocks are dropped: a rule inside an example is being quoted, not
/// given. Lines are split first so that a bulleted list of rules yields one
/// candidate per rule rather than one per paragraph.
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        for part in split_sentences(line) {
            if !part.trim().is_empty() {
                out.push(part);
            }
        }
    }
    out
}

fn split_sentences(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        let boundary = match c {
            '!' | '?' => chars.peek().is_none_or(|next| next.is_whitespace()),
            // A dot between digits is a version or a decimal, not a full stop.
            '.' => {
                chars.peek().is_none_or(|next| next.is_whitespace())
                    && !current
                        .chars()
                        .rev()
                        .nth(1)
                        .is_some_and(|prev| prev.is_ascii_digit())
            }
            _ => false,
        };
        if boundary {
            out.push(std::mem::take(&mut current));
            // Absorb the whitespace so the next sentence starts clean.
            while chars.peek().is_some_and(|next| next.is_whitespace()) {
                chars.next();
            }
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

/// Strip the markdown a user turn arrives wrapped in, and collapse whitespace.
fn clean(sentence: &str) -> String {
    // Markdown emphasis is stripped rather than quoted, so "**never** push" is
    // read as the rule it is. Leading list numbering goes too.
    let trimmed = sentence
        .replace(['*', '`', '_'], "")
        .trim()
        .trim_start_matches(['-', '#', '>', ' '])
        .trim()
        .to_string();
    let stripped = strip_leading_number(&trimmed);
    let mut out = String::with_capacity(stripped.len());
    let mut space = false;
    for c in stripped.chars() {
        if c.is_whitespace() {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(c);
    }
    out.trim_end_matches(['.', ' ']).trim().to_string()
}

/// Drop a leading `12.` / `12)` / `a)` marker from a list item.
fn strip_leading_number(text: &str) -> &str {
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || digits.len() > 3 {
        return text;
    }
    let rest = &text[digits.len()..];
    match rest.chars().next() {
        Some('.') | Some(')') => rest[1..].trim_start(),
        _ => text,
    }
}

/// Lowercased alphanumeric words, single-spaced.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_alphanumeric() {
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

/// The project's own directory names, as the scope vocabulary.
///
/// Exposed because the fold needs the same vocabulary the triage pass used: a
/// constraint and the chunk it governs have to be scoped by one function or the
/// routing is meaningless.
pub fn vocabulary(ledgers: &Ledgers) -> Vec<String> {
    subsystem_vocabulary(ledgers)
}

/// Subsystems a chunk of the session touched, from the ledger's file activity.
pub fn chunk_subsystems(
    ledgers: &Ledgers,
    start: EventIdx,
    end: EventIdx,
    vocabulary: &[String],
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for file in &ledgers.files {
        if file.last_evt < start || file.first_evt > end {
            continue;
        }
        for name in vocabulary {
            if file.path.split('/').any(|component| component == name) {
                out.push(name.clone());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The constraints that govern a chunk: the global ones, plus the scoped ones
/// whose scope the chunk touches.
///
/// This is Knowledge Triage's TypeDecompose (§3.3.2), which measures type-blind
/// partitioning at **93 % locality violations** and typed partitioning at
/// **0 %**, for a median replication overhead of 0 %. The failure it prevents is
/// concrete here: the fold walks 40 chunks, and a rule the user stated in
/// chunk 2 governs chunk 30 only if a model chose to re-emit it in every one of
/// the 28 intervening calls. Replication makes that arithmetic instead of
/// memory.
pub fn constraints_for<'a>(triage: &'a Triage, subsystems: &[String]) -> Vec<&'a Constraint> {
    triage
        .constraints
        .iter()
        .filter(|constraint| {
            constraint.scope.is_empty()
                || constraint
                    .scope
                    .iter()
                    .any(|scoped| subsystems.contains(scoped))
        })
        .collect()
}

/// Render the governing constraints for a chunk's prompt.
pub fn governing_block(constraints: &[&Constraint]) -> String {
    if constraints.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "Standing instructions the user gave, in their own words. They are binding \
         for this part of the work: do not contradict, weaken, or paraphrase them away, and \
         do not drop one because it does not come up in this chunk.\n",
    );
    for constraint in constraints {
        out.push_str(&format!(
            "- \"{}\" [evt {}]\n",
            constraint.text, constraint.evt
        ));
    }
    out
}

/// Put the triaged constraints into the state before anything else runs.
///
/// This is the paper's TypeCompact hard lane with the budget emergency removed:
/// sctxx can always afford its own constraints, so nothing here is ever dropped
/// to fit. They are ordinary `Constraint` items from this point on, which means
/// the fold may refine them, supersede them, or cite them — and the verifier
/// afterwards checks that it did not lose them.
pub fn seed(state: &mut FoldState, triage: &Triage) -> usize {
    for constraint in &triage.constraints {
        state.apply(
            "triage",
            &crate::pipeline::fold::ops::Op::Add {
                item: NewItem {
                    kind: ItemKind::Constraint,
                    text: constraint.text.clone(),
                    why: Some(format!(
                        "stated by the user; matched {}",
                        constraint.markers.join(", ")
                    )),
                    quote: Some(constraint.text.clone()),
                    rejected: Vec::new(),
                    sources: vec![EvtRange::new(constraint.evt, constraint.evt)],
                    confidence: Confidence::High,
                },
            },
        );
    }
    triage.constraints.len()
}

/// The paper's deterministic post-compaction verifier (§3.3.1), aimed at the
/// one thing it can check here: that each constraint the triage pass found is
/// still in the active state, verbatim.
///
/// It restores what it can rather than reporting a loss it could have undone.
/// The measured reason is in the paper: a run *without* this verifier reported
/// apparent 1.00 constraint recall while silently dropping a mean 57 % of the
/// constraints that should have been kept. A verifier that only reports would
/// reproduce that failure with better logging.
pub fn verify(state: &mut FoldState, triage: &Triage) -> GuardReport {
    let mut report = GuardReport {
        found: triage.constraints.len(),
        ..Default::default()
    };
    let present: Vec<String> = state
        .active_of(ItemKind::Constraint)
        .iter()
        .map(|item| normalize(&item.text))
        .collect();

    let mut missing: Vec<Constraint> = Vec::new();
    for constraint in &triage.constraints {
        let key = constraint.normalized();
        if present
            .iter()
            .any(|item| item == &key || item.contains(&key))
        {
            report.preserved += 1;
        } else {
            missing.push(constraint.clone());
        }
    }
    if missing.is_empty() {
        return report;
    }

    // Restore, then re-check rather than assume the restore worked.
    let salvage = Triage {
        constraints: missing.clone(),
        ..Triage::default()
    };
    seed(state, &salvage);
    let restored: Vec<String> = state
        .active_of(ItemKind::Constraint)
        .iter()
        .map(|item| normalize(&item.text))
        .collect();
    for constraint in missing {
        let key = constraint.normalized();
        if restored.iter().any(|item| item == &key) {
            report.restored += 1;
        } else {
            report.missing.push(constraint.text);
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_standing_rule_is_found_and_a_task_is_not() {
        // The distinction this module exists for: in a transcript the imperative
        // mood is what the user wants done now, not a standing rule.
        assert!(classify("push the progress and re-point the CLI", 1, &[]).is_none());
        assert!(classify("run the tests before you commit", 1, &[]).is_none());
        let rule = classify("never push directly to main", 1, &[]).expect("a rule");
        assert_eq!(rule.markers, vec!["prohibition"]);
    }

    #[test]
    fn questions_are_not_constraints() {
        assert!(classify("why did it never work?", 1, &[]).is_none());
        assert!(classify("never push to main?", 1, &[]).is_none());
        assert!(classify("could you always run that", 1, &[]).is_none());
    }

    #[test]
    fn code_fences_are_not_quoted_back_as_rules() {
        let message = "here is the file:\n```\n# never edit this\n```\nthanks";
        let found: Vec<String> = sentences(message);
        assert!(found.iter().all(|line| !line.contains("never edit")));
    }

    #[test]
    fn a_restated_rule_is_one_constraint() {
        let mut triage = Triage::default();
        let rules = [
            "never push to main",
            "Never push to main!",
            "never push to main.",
        ];
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        for (index, rule) in rules.iter().enumerate() {
            let constraint = classify(rule, index as EventIdx, &[]).expect("a rule");
            let key = constraint.normalized();
            if seen.contains_key(&key) {
                triage.duplicates += 1;
                continue;
            }
            seen.insert(key, triage.constraints.len());
            triage.constraints.push(constraint);
        }
        assert_eq!(triage.constraints.len(), 1);
        assert_eq!(triage.duplicates, 2);
    }

    #[test]
    fn scope_names_the_subsystems_a_rule_mentions() {
        let vocabulary = vec![
            "harness-runtime".to_string(),
            "desktop".to_string(),
            "cli".to_string(),
        ];
        assert_eq!(
            scope_of(
                "never modify the desktop schema without a migration",
                &vocabulary
            ),
            vec!["desktop".to_string()]
        );
        // Unnamed is global, which is the safe direction: it replicates.
        assert!(scope_of("always run the tests", &vocabulary).is_empty());
    }

    #[test]
    fn the_verifier_restores_a_constraint_the_fold_dropped() {
        let triage = Triage {
            constraints: vec![Constraint {
                text: "never push to main".to_string(),
                evt: 7,
                markers: vec!["prohibition".to_string()],
                scope: Vec::new(),
            }],
            messages: 1,
            sentences: 1,
            duplicates: 0,
            dropped: 0,
        };
        let mut state = FoldState::new();
        seed(&mut state, &triage);
        assert_eq!(state.active_of(ItemKind::Constraint).len(), 1);

        // The fold drops it — the exact loss the paper measures.
        let id = state.active_of(ItemKind::Constraint)[0].id.clone();
        state.apply(
            "c0",
            &crate::pipeline::fold::ops::Op::Drop {
                id,
                reason: "not relevant to this chunk".to_string(),
            },
        );
        assert!(state.active_of(ItemKind::Constraint).is_empty());

        let report = verify(&mut state, &triage);
        assert_eq!(report.found, 1);
        assert_eq!(report.preserved, 0);
        assert_eq!(report.restored, 1);
        assert!(report.is_clean());
        assert_eq!(state.active_of(ItemKind::Constraint).len(), 1);
    }

    #[test]
    fn word_boundaries_keep_never_out_of_whatever() {
        assert!(!contains_word("whatever happens", "never"));
        assert!(contains_word("never push", "never"));
        assert!(contains_word("this will never work", "never"));
    }

    #[test]
    fn normalization_survives_markdown_and_case() {
        assert_eq!(normalize("Never push to `main`!"), "never push to main");
        assert_eq!(normalize("  a\n b  "), "a b");
    }

    #[test]
    fn noise_is_not_a_user_turn() {
        assert!(classify("<system-reminder> you must never do this", 1, &[]).is_none());
    }

    #[test]
    fn a_description_that_contains_the_words_is_not_an_instruction() {
        // Every one of these was returned as a "binding user instruction" by the
        // first version of this classifier, on a real session.
        for description in [
            "CONTRACTS YOUR HOT-RELOAD / LIFECYCLE WORK MUST NOT BREAK",
            "GATES THAT MUST STAY GREEN",
            "intercept() and fetch.register() don't touch webServer, which is why only this one call",
            "DesktopSettingsSchema \"for validation\" and do not strip unknown keys",
            "(blends index + blends init acryl.demo) - do not hand-edit",
            "032 follow-up (6f468a8) - do not add a parallel mechanism for blend rows",
            "the schema must not be changed without a migration",
        ] {
            assert!(
                classify(description, 1, &[]).is_none(),
                "should not be a constraint: {description}"
            );
        }
    }

    #[test]
    fn a_rule_survives_its_politeness_and_its_markdown() {
        for rule in [
            "Please always run the formatter before you push",
            "and also, never commit secrets",
            "**never** push directly to main",
            "1. Do not edit generated files",
            "From now on, always commit the lockfile with the manifest",
            "Don't rename the exported symbols",
        ] {
            assert!(
                classify(rule, 1, &[]).is_some(),
                "should be a constraint: {rule}"
            );
        }
    }

    #[test]
    fn a_bullet_list_of_rules_yields_one_constraint_per_rule() {
        let found: Vec<String> =
            sentences("- never commit secrets\n- always run the tests\n- push before you leave");
        let rules: Vec<&String> = found
            .iter()
            .filter(|line| classify(line, 1, &[]).is_some())
            .collect();
        assert_eq!(rules.len(), 2, "got {rules:?}");
    }
}
