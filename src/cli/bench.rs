//! `sctxx bench` — measure whether a handoff artifact actually hands anything off.
//!
//! The benchmark is described in `src/pipeline/bench.rs` and in
//! `docs/BENCHMARK.md`. This command wires it to a real session and a real
//! backend and prints the table.
//!
//! It is deliberately able to lose. `none` and `tail` are in the arm list
//! because a win over doing nothing is not a win, and `tail` is the arm the
//! published evidence actually favours.

use super::{GlobalArgs, out, out_json};
use crate::error::{Error, Result};
use crate::ir::AgentKind;
use crate::llm::{Backend, CallRole, Request, Selection};
use crate::pipeline::bench::{self, Arm, Options, Question, Trial};
use crate::pipeline::{self, ExtractOptions, Mode};
use clap::Args;
use std::collections::BTreeMap;

/// `sctxx bench`
#[derive(Debug, Args)]
pub struct BenchArgs {
    /// One or more session references: `[claude|codex|pi:]<id|prefix|last>` or paths.
    #[arg(value_name = "REF", required = true)]
    references: Vec<String>,

    /// Backend that plays the successor agent. The benchmark measures a context,
    /// so it needs something to read it.
    #[arg(long, default_value = "auto")]
    llm: String,

    /// Which arms to run, comma separated: none, tail, artifact, retrieval.
    #[arg(long, default_value = "none,tail,artifact,retrieval")]
    arms: String,

    /// Questions per class: brief, deep, recent.
    #[arg(long, default_value_t = 6)]
    brief: usize,

    #[arg(long, default_value_t = 8)]
    deep: usize,

    #[arg(long, default_value_t = 4)]
    recent: usize,

    /// How many times the retrieval arm may ask for more transcript.
    #[arg(long, default_value_t = 3)]
    expansions: usize,

    /// Print what the successor answered for each question.
    #[arg(long)]
    show_answers: bool,

    #[arg(long)]
    any_project: bool,
}

pub fn run(args: &BenchArgs, global: &GlobalArgs) -> Result<i32> {
    // Arguments first, I/O second: a mistyped arm should cost a usage error, not
    // a failed connection to a provider the run was never going to use.
    let arms = parse_arms(&args.arms)?;
    let options = Options {
        brief: args.brief,
        deep: args.deep,
        recent: args.recent,
        expansions: args.expansions,
    };
    let selection = Selection::parse(&args.llm)?;
    let backend = crate::llm::build(&selection)?.ok_or_else(|| {
        Error::Usage(
            "sctxx bench needs a backend that can read the context: pass --llm api:<provider>, \
             api:compat/<model>, or cli:<agent>. Without one there is no successor agent and \
             nothing to measure."
                .into(),
        )
    })?;

    let mut trials: Vec<Trial> = Vec::new();
    let mut sessions: Vec<serde_json::Value> = Vec::new();

    for reference in &args.references {
        let resolved = crate::adapters::discovery::parse_reference(reference)?;
        let discovery = global.resolve_options(args.any_project, true);
        let summary = crate::adapters::discovery::resolve(&resolved, &discovery)?;

        // The artifact is produced exactly as `sctxx extract` produces it, so the
        // benchmark measures the shipped thing rather than a special build.
        let extract_options = ExtractOptions {
            llm: Selection::None,
            mode: Mode::Fast,
            verify: false,
            ..ExtractOptions::default()
        };
        global.note(&format!("extracting {}", summary.reference()));
        let extraction = pipeline::extract(&summary, &extract_options, &mut |stage, message| {
            global.note(&format!("[{stage}] {message}"))
        })?;

        let questions = bench::questions(&extraction.session, &extraction.ledgers, &options);
        if questions.is_empty() {
            global.note(&format!(
                "{}: no checkable questions could be derived; skipping",
                summary.reference()
            ));
            continue;
        }
        let artifact = extraction.markdown(&extract_options);
        let tail = crate::pipeline::mask::render(extraction.tail());

        global.note(&format!(
            "{}: {} question(s) × {} arm(s) via {}",
            summary.reference(),
            questions.len(),
            arms.len(),
            selection
        ));

        let mut session_trials = Vec::new();
        for arm in &arms {
            for question in &questions {
                let trial = run_trial(
                    backend.as_ref(),
                    *arm,
                    question,
                    &artifact,
                    &tail,
                    &extraction,
                    &options,
                );
                session_trials.push(trial);
            }
        }
        let scores = bench::score(&session_trials);
        sessions.push(serde_json::json!({
            "session": summary.reference(),
            "questions": questions.len(),
            "arms": scores.iter().map(|(arm, score)| {
                (arm.label().to_string(), serde_json::json!({
                    "asked": score.asked,
                    "correct": score.correct,
                    "accuracy": score.accuracy(),
                    "tokens": score.tokens,
                    "tokens_per_correct": score.tokens_per_correct(),
                    "expansions": score.expansions,
                    "by_class": score.by_class,
                }))
            }).collect::<BTreeMap<_, _>>(),
        }));
        trials.extend(session_trials);
    }

    if trials.is_empty() {
        return Err(Error::Usage(
            "no questions could be derived from any of the given sessions".into(),
        ));
    }

    let scores = bench::score(&trials);
    if global.json {
        out_json(&serde_json::json!({
            "schema": "sctxx.bench/v1",
            "backend": selection.to_string(),
            "questions": trials.len() / arms.len().max(1),
            "arms": arms.iter().map(|arm| arm.label()).collect::<Vec<_>>(),
            "overall": scores.iter().map(|(arm, score)| {
                (arm.label().to_string(), serde_json::json!({
                    "asked": score.asked,
                    "correct": score.correct,
                    "accuracy": score.accuracy(),
                    "tokens": score.tokens,
                    "tokens_per_correct": score.tokens_per_correct(),
                    "expansions": score.expansions,
                    "by_class": score.by_class,
                }))
            }).collect::<BTreeMap<_, _>>(),
            "sessions": sessions,
        }))?;
        return Ok(0);
    }

    out(&render(&scores, &trials, &selection.to_string()));
    if args.show_answers {
        let mut dump = String::from("\nanswers\n");
        for trial in &trials {
            dump.push_str(&format!(
                "\n[{}] {} ({}) {} — {}\n",
                trial.arm.label(),
                trial.question,
                trial.class.label(),
                if trial.correct { "correct" } else { "WRONG" },
                trial
                    .answer
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            ));
        }
        out(&dump);
    }
    Ok(0)
}

/// Ask one question under one arm, including any retrieval rounds.
fn run_trial(
    backend: &dyn Backend,
    arm: Arm,
    question: &Question,
    artifact: &str,
    tail: &str,
    extraction: &pipeline::Extraction,
    options: &Options,
) -> Trial {
    let context = match arm {
        Arm::None => String::new(),
        Arm::Tail => tail.to_string(),
        Arm::Artifact | Arm::Retrieval => artifact.to_string(),
    };
    let mut conversation = String::new();
    let mut tokens = 0usize;
    let mut expansions = 0usize;
    let mut answer = String::new();

    for round in 0..=options.expansions {
        let user = prompt(arm, &context, &conversation, question, round == 0);
        tokens += crate::vendor::codex::truncate::approx_token_count(&user);
        let request = Request {
            role: CallRole::Probe,
            system: system(arm).to_string(),
            user,
            json_schema: None,
            max_output_tokens: 400,
            temperature: Some(0.0),
        };
        answer = match backend.complete(&request) {
            Ok(response) => response.text,
            Err(error) => {
                // A backend failure is a failed question, not a crashed
                // benchmark: one arm losing a call must not lose the run.
                conversation.push_str(&format!("\n(error from the backend: {error})\n"));
                break;
            }
        };
        if !arm.can_expand() || expansions >= options.expansions {
            break;
        }
        let Some(range) = bench::requested_range(&answer) else {
            break;
        };
        expansions += 1;
        let events = bench::expand(&extraction.session, range);
        tokens += crate::vendor::codex::truncate::approx_token_count(&events);
        conversation.push_str(&format!(
            "\nYou asked for evt {}..{}:\n{}\n",
            range.start, range.end, events
        ));
    }

    Trial {
        question: question.id.clone(),
        class: question.class,
        arm,
        correct: question.answered_by(&answer),
        tokens,
        expansions,
        answer,
    }
}

fn system(arm: Arm) -> &'static str {
    if arm.can_expand() {
        "You are taking over a coding task from another agent. You are given a handoff artifact. \
         Answer the question from it. If the artifact does not contain the answer, you may ask for \
         the raw events by replying with a single line `EXPAND <start>..<end>`; you will be given \
         those events and asked again. You may do that a few times. Otherwise answer in one short \
         sentence, and say NOT FOUND if the context does not contain it."
    } else {
        "You are taking over a coding task from another agent. Answer the question using only the \
         context you are given. Answer in one short sentence, and say NOT FOUND if the context does \
         not contain the answer. Do not guess."
    }
}

fn prompt(arm: Arm, context: &str, conversation: &str, question: &Question, first: bool) -> String {
    if arm == Arm::None {
        return format!("QUESTION: {}\n", question.prompt);
    }
    let mut out = String::new();
    if first {
        out.push_str("# CONTEXT\n\n");
        out.push_str(context);
        out.push_str("\n\n");
    }
    out.push_str(conversation);
    out.push_str(&format!("\nQUESTION: {}\n", question.prompt));
    out
}

fn parse_arms(text: &str) -> Result<Vec<Arm>> {
    let mut arms = Vec::new();
    for name in text.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        let arm = match name {
            "none" => Arm::None,
            "tail" => Arm::Tail,
            "artifact" => Arm::Artifact,
            "retrieval" | "artifact+retrieval" => Arm::Retrieval,
            other => {
                return Err(Error::Usage(format!(
                    "unknown arm `{other}` (expected none, tail, artifact, or retrieval)"
                )));
            }
        };
        if !arms.contains(&arm) {
            arms.push(arm);
        }
    }
    if arms.is_empty() {
        return Err(Error::Usage("no arms selected".into()));
    }
    Ok(arms)
}

/// The table. Accuracy first, then what it cost, then per class — because an arm
/// that is five points better at four times the price is a different claim.
fn render(scores: &BTreeMap<Arm, bench::ArmScore>, trials: &[Trial], backend: &str) -> String {
    let classes: Vec<bench::Class> = {
        let mut seen = Vec::new();
        for trial in trials {
            if !seen.contains(&trial.class) {
                seen.push(trial.class);
            }
        }
        seen
    };
    let mut out = String::new();
    out.push_str("sctxx handoff benchmark (sctxx.bench/v1)\n\n");
    out.push_str(&format!("successor: {backend}\n"));
    out.push_str(&format!(
        "questions: {}\n\n",
        scores.values().map(|s| s.asked).max().unwrap_or(0)
    ));

    out.push_str(&format!(
        "{:<20} {:>8} {:>10} {:>12} {:>12}\n",
        "arm", "correct", "accuracy", "tokens", "tok/correct"
    ));
    for (arm, score) in scores {
        out.push_str(&format!(
            "{:<20} {:>8} {:>9.0}% {:>12} {:>12}\n",
            arm.label(),
            format!("{}/{}", score.correct, score.asked),
            score.accuracy() * 100.0,
            score.tokens,
            score
                .tokens_per_correct()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "—".into()),
        ));
    }

    out.push_str("\nby question class\n");
    out.push_str(&format!(
        "{:<20} {}\n",
        "arm",
        classes
            .iter()
            .map(|class| format!("{:>10}", class.label()))
            .collect::<Vec<_>>()
            .join("")
    ));
    for (arm, score) in scores {
        let cells = classes
            .iter()
            .map(|class| match score.class_accuracy(*class) {
                Some(value) => format!("{:>9.0}%", value * 100.0),
                None => format!("{:>10}", "—"),
            })
            .collect::<Vec<_>>()
            .join("");
        out.push_str(&format!("{:<20} {cells}\n", arm.label()));
    }

    let expansions: usize = scores.values().map(|score| score.expansions).sum();
    if expansions > 0 {
        out.push_str(&format!(
            "\nthe retrieval arm asked for more transcript {expansions} time(s)\n"
        ));
    }
    out.push_str(
        "\nThis measures whether a fresh agent can answer questions about the session from each \
         context. It does not measure whether it resolves an issue — see docs/BENCHMARK.md.\n",
    );
    out
}

/// Kept so the command compiles under `--no-default-features`, where the LLM
/// backends are absent and the benchmark cannot run.
#[allow(dead_code)]
fn unavailable() -> Error {
    Error::Usage("sctxx bench needs the `api` or `cli-backends` feature".into())
}

/// The agent slug only matters for the reference parser.
#[allow(dead_code)]
fn _agent_of(reference: &str) -> Option<AgentKind> {
    reference.split(':').next().and_then(AgentKind::from_slug)
}
