//! The extraction form.
//!
//! Every field is read from clap's own definition of `sctxx extract`
//! ([`ExtractArgs::command`]), and submitting goes back through clap
//! ([`ExtractArgs::parse_argv`]). That is the entire design, and it is what
//! makes SC-004 checkable rather than aspirational: the form cannot grow a
//! control the CLI does not have, cannot lose one it does, and cannot accept a
//! value the CLI would reject — because clap is the one deciding all three.
//!
//! A form-only opinion is limited to *presentation*, and is confined to
//! [`Control`]: clap says whether a flag is boolean and which values it accepts,
//! so the widget is derived rather than chosen.

use crate::cli::GlobalArgs;
use crate::cli::extract::ExtractArgs;
use crate::error::Result;
use crate::pipeline::ExtractOptions;
use clap::ArgAction;

/// Flags the form does not offer, and why.
///
/// Each of these describes the *command line* rather than what to extract: they
/// are about stdout and stderr, and a full-screen interface writes neither
/// (FR-004 — the TUI never puts a payload on the terminal). Offering them would
/// be offering controls that do nothing, which is worse than not having them.
///
/// The test asserts this list against clap in both directions, so a new flag
/// forces a decision and a stale entry here fails rather than silently dropping
/// something.
pub const CLI_ONLY: &[(&str, &str)] = &[
    ("format", "describes stdout; the TUI writes files or panes"),
    (
        "progress",
        "describes stderr; progress is drawn in the pane",
    ),
    (
        "dry_run",
        "prints a plan and exits; the pane shows it before running",
    ),
];

/// Where the artifact goes unless the developer says otherwise.
///
/// The CLI defaults `--out` to stdout, which is the right answer for a command
/// and the wrong one in a full-screen interface. This is a *value* the developer
/// sees in the form before pressing enter, not a hidden default.
const DEFAULT_DESTINATION: &str = ".sctxx/";

/// How one field is edited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    /// A boolean flag: on or off.
    Toggle,
    /// One of a fixed set, in clap's order.
    Choice(Vec<String>),
    /// Free text: numbers, paths, and the flags whose values are dynamic.
    Text,
}

/// One control in the form, derived from one clap argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// clap's id. Also the stable key for tests and for setting a value.
    pub id: String,
    /// How a developer writes it, e.g. `--max-bad-lines`.
    pub flag: String,
    /// clap's help text, on one line.
    pub help: String,
    pub control: Control,
    pub value: String,
}

impl Field {
    /// True when this flag is on. Only meaningful for [`Control::Toggle`].
    pub fn is_on(&self) -> bool {
        self.value == "true"
    }

    /// The value as the form draws it.
    ///
    /// The shape says what can be done with it: a bracketed box is a switch, and
    /// arrows mean there are other values. A reader should not have to type at a
    /// field to discover that it does not take typing.
    pub fn shown(&self) -> String {
        match &self.control {
            Control::Toggle => {
                if self.is_on() {
                    "[x]".to_string()
                } else {
                    "[ ]".to_string()
                }
            }
            Control::Choice(_) => format!("\u{25c0} {} \u{25b6}", self.value),
            _ if self.value.is_empty() => "(unset)".to_string(),
            _ => self.value.clone(),
        }
    }

    /// How this field is changed, for the line under it.
    pub fn affordance(&self) -> &'static str {
        match self.control {
            Control::Toggle => "space or \u{2190}/\u{2192} switches it",
            Control::Choice(_) => "\u{2190}/\u{2192} chooses",
            Control::Text => "type to edit, backspace to delete",
        }
    }
}

/// The whole form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    fields: Vec<Field>,
    /// Which row is focused. `fields.len()` is the run row at the bottom, so
    /// running is something the reader navigates to rather than a meaning hiding
    /// behind `enter` on whichever field happened to be focused.
    focus: usize,
    /// What just happened, when nothing did. Silence is the worst answer to a
    /// keypress.
    hint: Option<String>,
    /// The session this form will extract.
    reference: String,
}

impl Form {
    /// Build the form for one session, from clap's definition of `extract`.
    pub fn new(reference: &str) -> Self {
        // `--help` and `--version` are not arguments of the command in the sense
        // that matters here; clap adds them to the built command.
        let command = ExtractArgs::command();
        let fields = command
            .get_arguments()
            // A positional is the session itself, and the form already knows it.
            .filter(|arg| arg.get_long().is_some())
            .filter(|arg| arg.get_id() != "help" && arg.get_id() != "version")
            .filter(|arg| !CLI_ONLY.iter().any(|(id, _)| *id == arg.get_id().as_str()))
            .map(field_from)
            .collect();

        Self {
            fields,
            focus: 0,
            hint: None,
            reference: reference.to_string(),
        }
    }

    /// How many rows the form has, including the action at the bottom.
    pub fn rows(&self) -> usize {
        self.fields.len() + 1
    }

    /// True when the run row is focused.
    pub fn on_run_row(&self) -> bool {
        self.focus >= self.fields.len()
    }

    /// What just happened, when nothing did.
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }

    pub fn set_hint(&mut self, hint: Option<String>) {
        self.hint = hint;
    }

    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    pub fn focus(&self) -> usize {
        self.focus
    }

    pub fn focused(&self) -> Option<&Field> {
        self.fields.get(self.focus)
    }

    /// Move between fields, clamped at both ends.
    pub fn move_focus(&mut self, delta: isize) {
        let last = self.rows().saturating_sub(1) as isize;
        let next = self.focus as isize + delta;
        self.focus = next.clamp(0, last) as usize;
        self.hint = None;
    }

    /// The first row drawn, so the focused row is always on screen.
    pub fn scroll_for_rows(&self, height: usize) -> usize {
        if height == 0 || self.rows() <= height {
            return 0;
        }
        let first = self.focus.saturating_sub(height / 2);
        first.min(self.rows() - height)
    }

    /// Space or enter on the focused field: flip a toggle, step a choice on.
    pub fn activate(&mut self) {
        self.hint = None;
        match self.focused().map(|field| field.control.clone()) {
            Some(Control::Toggle) => self.step(1),
            Some(Control::Choice(_)) => self.step(1),
            _ => {}
        }
    }

    /// Step the focused field's value: a toggle flips, a choice moves by one
    /// and wraps, text does nothing.
    pub fn step(&mut self, delta: isize) {
        let Some(field) = self.fields.get(self.focus) else {
            return;
        };
        let control = field.control.clone();
        match control {
            Control::Toggle => {
                let on = field.is_on();
                self.set(&field.id.clone(), if on { "false" } else { "true" });
            }
            Control::Choice(values) => {
                if values.is_empty() {
                    return;
                }
                let current = values.iter().position(|value| *value == field.value);
                let last = values.len() - 1;
                let next = match current {
                    Some(index) if delta > 0 => (index + 1) % values.len(),
                    Some(0) if delta < 0 => last,
                    Some(index) if delta < 0 => index - 1,
                    Some(_) => return,
                    None => 0,
                };
                self.set(&field.id.clone(), &values[next].clone());
            }
            Control::Text => {}
        }
    }

    /// Type into the focused field, if it takes text.
    pub fn push_char(&mut self, character: char) {
        if self.on_run_row() {
            self.hint =
                Some("this row runs the extraction; tab back to a field to change it".into());
            return;
        }
        let Some(field) = self.fields.get(self.focus) else {
            return;
        };
        if !matches!(field.control, Control::Text) {
            // The silent no-op that made this form feel broken. Say what the
            // field is rather than ignoring the keystroke.
            let (flag, affordance) = (field.flag.clone(), field.affordance());
            self.hint = Some(format!("{flag} is not typed into: {affordance}"));
            return;
        }
        self.hint = None;
        let id = field.id.clone();
        let mut value = field.value.clone();
        value.push(character);
        self.set(&id, &value);
    }

    /// Delete the last character of the focused text field.
    pub fn pop_char(&mut self) {
        let Some(field) = self.fields.get(self.focus) else {
            return;
        };
        if !matches!(field.control, Control::Text) {
            return;
        }
        self.hint = None;
        let id = field.id.clone();
        let mut value = field.value.clone();
        value.pop();
        self.set(&id, &value);
    }

    /// Set a field by clap's id. Returns false when there is no such field.
    pub fn set(&mut self, id: &str, value: &str) -> bool {
        match self.fields.iter_mut().find(|field| field.id == id) {
            Some(field) => {
                field.value = value.to_string();
                true
            }
            None => false,
        }
    }

    /// Read a field by clap's id.
    pub fn get(&self, id: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|field| field.id == id)
            .map(|field| field.value.as_str())
    }

    /// The argv this form stands for. Text fields left empty are omitted, so
    /// clap applies its own default rather than being handed an empty string.
    ///
    /// The leading element is the program name clap expects, not a subcommand:
    /// [`ExtractArgs::command`] *is* the `extract` command, so its positional
    /// really is the session reference.
    pub fn argv(&self) -> Vec<String> {
        let mut argv = vec!["sctxx".to_string()];
        for field in &self.fields {
            match field.control {
                Control::Toggle => {
                    if field.is_on() {
                        argv.push(field.flag.clone());
                    }
                }
                _ => {
                    if !field.value.is_empty() {
                        argv.push(field.flag.clone());
                        argv.push(field.value.clone());
                    }
                }
            }
        }
        argv.push(self.reference.clone());
        argv
    }

    /// Validate through clap and convert, exactly as the command line would.
    ///
    /// A value the CLI would reject is rejected here by the same parser, which
    /// is the point: the form has no validation of its own to get wrong.
    pub fn options(&self, global: &GlobalArgs) -> Result<ExtractOptions> {
        ExtractArgs::parse_argv(self.argv())?.options(global)
    }
}

/// Build one field from one clap argument.
fn field_from(arg: &clap::Arg) -> Field {
    let id = arg.get_id().to_string();
    let flag = format!("--{}", arg.get_long().unwrap_or_default());
    let help = arg
        .get_help()
        .map(|help| help.to_string())
        .unwrap_or_default()
        .replace('\n', " ");
    let toggle = matches!(arg.get_action(), ArgAction::SetTrue);
    let choices: Vec<String> = arg
        .get_possible_values()
        .iter()
        .map(|value| value.get_name().to_string())
        .collect();
    let control = if toggle {
        Control::Toggle
    } else if choices.is_empty() {
        Control::Text
    } else {
        Control::Choice(choices)
    };
    let default = arg
        .get_default_values()
        .first()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let value = if toggle {
        "false".to_string()
    } else if id == "out" && default.is_empty() {
        DEFAULT_DESTINATION.to_string()
    } else {
        default
    };

    Field {
        id,
        flag,
        help,
        control,
        value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> Form {
        Form::new("claude:1367d688")
    }

    #[test]
    fn every_flag_the_cli_has_is_a_field_and_nothing_else_is() {
        // This is SC-004. The form is built from clap, so the test asserts the
        // property that makes it true rather than a hand-kept list: the set of
        // flags on screen equals the set of flags clap defines.
        let command = ExtractArgs::command();
        let mut from_clap: Vec<String> = command
            .get_arguments()
            .filter(|arg| arg.get_long().is_some())
            .filter(|arg| arg.get_id() != "help" && arg.get_id() != "version")
            .map(|arg| arg.get_id().to_string())
            .collect();
        let mut from_form: Vec<String> = form()
            .fields()
            .iter()
            .map(|field| field.id.clone())
            .collect();
        from_clap.sort();
        from_form.sort();
        // Every flag is offered except the ones about a pipe, which a
        // full-screen interface does not have.
        let expected: Vec<String> = from_clap
            .iter()
            .filter(|id| !CLI_ONLY.iter().any(|(excluded, _)| excluded == *id))
            .cloned()
            .collect();
        assert_eq!(
            expected, from_form,
            "the form and `sctxx extract` must expose the same flags, less the pipe-only ones"
        );
        // And every exclusion names a real flag, so a typo cannot quietly drop
        // something instead of failing here.
        for (id, reason) in CLI_ONLY {
            assert!(
                from_clap.contains(&id.to_string()),
                "`{id}` is excluded from the form but is not a flag"
            );
            assert!(!reason.is_empty(), "`{id}` is excluded without a reason");
        }
    }

    #[test]
    fn the_only_argument_without_a_flag_is_the_session_reference() {
        // The reference is positional and comes from the browser selection, so
        // the form deliberately does not expose it. A *new* positional would
        // otherwise be silently missing from the form, so it fails here.
        let command = ExtractArgs::command();
        let positionals: Vec<String> = command
            .get_arguments()
            .filter(|arg| arg.get_long().is_none() && arg.get_id() != "help")
            .map(|arg| arg.get_id().to_string())
            .collect();
        assert_eq!(
            positionals,
            vec!["reference".to_string()],
            "a new positional argument needs a decision about the form"
        );
    }

    #[test]
    fn the_argv_round_trips_through_clap() {
        let global = GlobalArgs::default();
        let options = form()
            .options(&global)
            .expect("an untouched form must be valid");
        // The defaults clap advertises have to survive the trip.
        assert_eq!(options.budget, 8_000);
        assert_eq!(options.tail_tokens, 12_000);
        assert_eq!(options.chunk_tokens, 24_000);
        assert!(options.verify, "verification is on unless --no-verify");
    }

    #[test]
    fn a_toggle_emits_its_flag_only_when_it_is_on() {
        let mut form = form();
        assert!(form.set("no_verify", "false"));
        assert!(
            !form.argv().contains(&"--no-verify".to_string()),
            "an off toggle must not appear in argv"
        );
        form.set("no_verify", "true");
        assert!(form.argv().contains(&"--no-verify".to_string()));
    }

    #[test]
    fn a_choice_field_steps_through_the_values_clap_lists() {
        let mut form = form();
        // `--mode` is declared with a value_parser, so clap is the source of
        // the list rather than a table in the form.
        let mode = form
            .fields()
            .iter()
            .find(|field| field.id == "mode")
            .expect("a mode field");
        assert_eq!(
            mode.control,
            Control::Choice(vec![
                "fast".to_string(),
                "standard".to_string(),
                "full".to_string()
            ])
        );
        // `fast` is the default: folding every chunk in sequence is hours on a
        // real session, so `standard` is the deliberate choice, not the default.
        assert_eq!(form.get("mode"), Some("fast"));

        let index = form
            .fields()
            .iter()
            .position(|field| field.id == "mode")
            .expect("index");
        form.move_focus(index as isize);
        form.step(1);
        assert_eq!(form.get("mode"), Some("standard"));
        form.step(1);
        assert_eq!(form.get("mode"), Some("full"));
        form.step(1);
        assert_eq!(form.get("mode"), Some("fast"), "a choice wraps");
        form.step(-1);
        assert_eq!(form.get("mode"), Some("full"), "and steps back");
    }

    #[test]
    fn a_toggle_is_flipped_rather_than_stepped() {
        let mut form = form();
        let index = form
            .fields()
            .iter()
            .position(|field| field.id == "include_sidechains")
            .expect("index");
        form.move_focus(index as isize);
        form.activate();
        assert_eq!(form.get("include_sidechains"), Some("true"));
        form.activate();
        assert_eq!(form.get("include_sidechains"), Some("false"));
    }

    #[test]
    fn a_value_the_cli_would_reject_is_rejected_by_clap() {
        let mut form = form();
        form.set("max_bad_lines", "not-a-number");
        let error = form
            .options(&GlobalArgs::default())
            .expect_err("clap must refuse it");
        assert_eq!(error.exit_code(), 2);
        // The message names the flag, because clap produced it.
        assert!(error.to_string().contains("max-bad-lines"), "{error}");
    }

    #[test]
    fn text_fields_take_characters_and_choices_do_not() {
        let mut form = form();
        let focus = form
            .fields()
            .iter()
            .position(|field| field.id == "focus")
            .expect("index");
        form.move_focus(focus as isize);
        for character in "rename the parser".chars() {
            form.push_char(character);
        }
        assert_eq!(form.get("focus"), Some("rename the parser"));
        form.pop_char();
        assert_eq!(form.get("focus"), Some("rename the parse"));

        // Typing at a choice must not corrupt it.
        let mode = form
            .fields()
            .iter()
            .position(|field| field.id == "mode")
            .expect("index");
        form.move_focus(mode as isize - focus as isize);
        let before = form.get("mode").map(str::to_string);
        form.push_char('x');
        assert_eq!(form.get("mode").map(str::to_string), before);
    }

    #[test]
    fn the_focused_field_is_always_inside_the_drawn_window() {
        let mut form = form();
        let height = 10;
        assert!(form.rows() > height, "the fixture must overflow");
        for focus in [0, 1, 5, form.rows() - 1] {
            form.move_focus(focus as isize - form.focus() as isize);
            let first = form.scroll_for_rows(height);
            assert!(
                form.focus() >= first && form.focus() < first + height,
                "focus {} outside window {first}..{}",
                form.focus(),
                first + height
            );
            assert!(first + height <= form.rows(), "never scroll past the end");
        }
        // A window taller than the form, and a degenerate one, both start at 0.
        assert_eq!(form.scroll_for_rows(form.rows()), 0);
        assert_eq!(form.scroll_for_rows(0), 0);
    }

    #[test]
    fn focus_is_clamped_at_both_ends() {
        let mut form = form();
        form.move_focus(-10);
        assert_eq!(form.focus(), 0);
        form.move_focus(1_000);
        // The last row is the run action, not a field.
        assert_eq!(form.focus(), form.rows() - 1);
        assert!(form.on_run_row());
        form.move_focus(10);
        assert_eq!(form.focus(), form.rows() - 1);
    }

    #[test]
    fn the_destination_is_prefilled_and_visible() {
        // A full-screen interface has no stdout to default to, so the form
        // offers the ordinary destination as a value the developer can see and
        // change before running.
        let form = form();
        assert_eq!(form.get("out"), Some(DEFAULT_DESTINATION));
        assert!(form.argv().contains(&DEFAULT_DESTINATION.to_string()));
    }

    #[test]
    fn an_empty_text_field_is_omitted_so_clap_applies_its_own_default() {
        let mut form = form();
        form.set("focus", "");
        form.set("repo", "");
        let argv = form.argv();
        assert!(!argv.contains(&"--focus".to_string()));
        assert!(!argv.contains(&"--repo".to_string()));
        form.options(&GlobalArgs::default())
            .expect("omitting optional flags must stay valid");
    }
}
