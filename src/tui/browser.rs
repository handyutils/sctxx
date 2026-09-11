//! The session browser's state, and the filtering that feeds it.
//!
//! Everything here is pure: no terminal, no clock except one value passed in,
//! no rendering. That is deliberate — the interesting behaviour of this screen
//! (which sessions survive a filter, how a query ranks them, where the cursor
//! lands) is the part worth testing, and it is the part that would otherwise be
//! buried inside an event loop that needs a TTY to run.

use crate::adapters::discovery::SessionSummary;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use std::sync::OnceLock;

/// The scorer is stateless configuration, so one shared instance beats a field
/// on every browser — and it keeps `Browser` free of a type that has no
/// `Debug`, which the crate lints require.
fn matcher() -> &'static SkimMatcherV2 {
    static MATCHER: OnceLock<SkimMatcherV2> = OnceLock::new();
    MATCHER.get_or_init(SkimMatcherV2::default)
}

/// How far back to look, as a rolling window.
///
/// Rolling windows rather than calendar days on purpose: the only clock input
/// available without a date crate is the file mtime, and "last 7 days" is both
/// cheaper and less surprising than "since Monday".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recency {
    Any,
    Day,
    Week,
    Month,
}

impl Recency {
    pub fn label(self) -> &'static str {
        match self {
            Recency::Any => "any time",
            Recency::Day => "last 24h",
            Recency::Week => "last 7d",
            Recency::Month => "last 30d",
        }
    }

    /// The next window in the cycle, so one key walks all four.
    pub fn next(self) -> Self {
        match self {
            Recency::Any => Recency::Day,
            Recency::Day => Recency::Week,
            Recency::Week => Recency::Month,
            Recency::Month => Recency::Any,
        }
    }

    fn window(self) -> Option<u64> {
        match self {
            Recency::Any => None,
            Recency::Day => Some(24 * 60 * 60),
            Recency::Week => Some(7 * 24 * 60 * 60),
            Recency::Month => Some(30 * 24 * 60 * 60),
        }
    }
}

/// The browser: everything the list screen knows.
#[derive(Debug)]
pub struct Browser {
    sessions: Vec<SessionSummary>,
    agent: Option<&'static str>,
    recency: Recency,
    this_project: bool,
    query: String,
    /// Indices into `sessions`, in display order.
    visible: Vec<usize>,
    selected: usize,
    /// Directories that count as "this project".
    project: Option<String>,
    now: u64,
}

impl Browser {
    /// `now` is seconds since the Unix epoch, so the caller owns the clock.
    pub fn new(sessions: Vec<SessionSummary>, now: u64, project: Option<String>) -> Self {
        let mut browser = Self {
            sessions,
            agent: None,
            recency: Recency::Any,
            this_project: false,
            query: String::new(),
            visible: Vec::new(),
            selected: 0,
            project,
            now,
        };
        browser.refilter();
        browser
    }

    pub fn sessions(&self) -> &[SessionSummary] {
        &self.sessions
    }

    /// The rows to draw, in order.
    pub fn visible(&self) -> impl Iterator<Item = &SessionSummary> {
        self.visible.iter().filter_map(|i| self.sessions.get(*i))
    }

    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    pub fn selected_row(&self) -> usize {
        self.selected
    }

    pub fn selected(&self) -> Option<&SessionSummary> {
        self.visible
            .get(self.selected)
            .and_then(|i| self.sessions.get(*i))
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn agent_filter(&self) -> Option<&'static str> {
        self.agent
    }

    pub fn recency(&self) -> Recency {
        self.recency
    }

    pub fn this_project(&self) -> bool {
        self.this_project
    }

    pub fn total(&self) -> usize {
        self.sessions.len()
    }

    /// The agents present in the data, in the order they first appear.
    pub fn agents(&self) -> Vec<&'static str> {
        let mut seen: Vec<&'static str> = Vec::new();
        for session in &self.sessions {
            if !seen.contains(&session.agent) {
                seen.push(session.agent);
            }
        }
        seen
    }

    // ---- filters ----------------------------------------------------------

    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.refilter();
    }

    pub fn push_query(&mut self, c: char) {
        self.query.push(c);
        self.refilter();
    }

    pub fn pop_query(&mut self) {
        self.query.pop();
        self.refilter();
    }

    /// Cycle to the next agent filter, ending back at "all".
    pub fn cycle_agent(&mut self) {
        let agents = self.agents();
        let next = match self.agent {
            None => agents.first().copied(),
            Some(current) => {
                let at = agents.iter().position(|a| *a == current);
                match at {
                    Some(i) if i + 1 < agents.len() => Some(agents[i + 1]),
                    _ => None,
                }
            }
        };
        self.agent = next;
        self.refilter();
    }

    pub fn cycle_recency(&mut self) {
        self.recency = self.recency.next();
        self.refilter();
    }

    pub fn toggle_project(&mut self) {
        self.this_project = !self.this_project;
        self.refilter();
    }

    /// A one-line description of the active filters, for the status bar.
    pub fn filter_summary(&self) -> String {
        let mut parts = vec![match self.agent {
            Some(agent) => agent.to_string(),
            None => "all agents".to_string(),
        }];
        parts.push(self.recency.label().to_string());
        if self.this_project {
            parts.push("this project".to_string());
        }
        if !self.query.is_empty() {
            parts.push(format!("“{}”", self.query));
        }
        parts.join(" · ")
    }

    // ---- movement ---------------------------------------------------------

    pub fn move_by(&mut self, delta: isize) {
        if self.visible.is_empty() {
            self.selected = 0;
            return;
        }
        let last = self.visible.len() - 1;
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, last as isize) as usize;
    }

    pub fn select(&mut self, row: usize) {
        self.selected = row.min(self.visible.len().saturating_sub(1));
    }

    // ---- the filter itself ------------------------------------------------

    /// Recompute `visible` from the filters and the query, and keep the cursor
    /// on the same session if it survived.
    fn refilter(&mut self) {
        let keep = self.selected().map(|s| (s.agent, s.id.clone()));
        let cutoff = self.recency.window().map(|w| self.now.saturating_sub(w));

        let mut ranked: Vec<(i64, usize)> = Vec::new();
        let mut unranked: Vec<usize> = Vec::new();

        for (index, session) in self.sessions.iter().enumerate() {
            if let Some(agent) = self.agent
                && session.agent != agent
            {
                continue;
            }
            if let Some(cutoff) = cutoff
                && session.mtime < cutoff
            {
                continue;
            }
            if self.this_project && !self.in_project(session) {
                continue;
            }

            if self.query.is_empty() {
                unranked.push(index);
                continue;
            }
            if let Some(score) = self.score(session) {
                ranked.push((score, index));
            }
        }

        if self.query.is_empty() {
            // `list_all` already sorted by recency; keep that order.
            self.visible = unranked;
        } else {
            // Highest score first; ties broken by the existing (recency) order,
            // so the ranking is stable and never depends on map iteration.
            ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            self.visible = ranked.into_iter().map(|(_, index)| index).collect();
        }

        self.selected = match keep {
            Some((agent, id)) => self
                .visible
                .iter()
                .position(|i| {
                    let s = &self.sessions[*i];
                    s.agent == agent && s.id == id
                })
                .unwrap_or(0),
            None => 0,
        };
        self.selected = self.selected.min(self.visible.len().saturating_sub(1));
    }

    /// Whether a session belongs to the current directory.
    ///
    /// A session that recorded no cwd is *not* in this project: including it
    /// would make the filter quietly useless on exactly the sessions a
    /// developer cannot place.
    fn in_project(&self, session: &SessionSummary) -> bool {
        match (self.project.as_deref(), session.cwd.as_deref()) {
            (Some(project), Some(cwd)) => cwd == project,
            _ => false,
        }
    }

    /// The fuzzy score for one session, or `None` when it does not match.
    ///
    /// A query that looks like an id or a path is matched exactly first and
    /// ranked above everything else: when a developer pastes an id, they mean
    /// that session, not a fuzzy coincidence.
    fn score(&self, session: &SessionSummary) -> Option<i64> {
        let query = self.query.trim();
        if query.is_empty() {
            return Some(0);
        }
        if session.id.starts_with(query) || session.path.to_string_lossy().contains(query) {
            return Some(i64::MAX);
        }
        let haystack = format!(
            "{} {} {} {}",
            session.title.as_deref().unwrap_or_default(),
            session.first_message.as_deref().unwrap_or_default(),
            session.id,
            session.cwd.as_deref().unwrap_or_default(),
        );
        matcher().fuzzy_match(&haystack, query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const HOUR: u64 = 60 * 60;
    const NOW: u64 = 1_800_000_000;

    fn session(agent: &'static str, id: &str, age_hours: u64, cwd: &str) -> SessionSummary {
        SessionSummary {
            agent,
            id: id.to_string(),
            path: PathBuf::from(format!("/store/{id}.jsonl")),
            cwd: Some(cwd.to_string()),
            started_at: None,
            ended_at: None,
            lines: 10,
            bytes: 100,
            title: Some(format!("{id} title")),
            first_message: Some(format!("work on {id}")),
            mtime: NOW - age_hours * HOUR,
        }
    }

    fn fixture() -> Vec<SessionSummary> {
        // Deliberately newest-first, as `list_all` produces.
        vec![
            session("claude", "claude-new", 1, "/code/acryl"),
            session("codex", "codex-mid", 50, "/code/acryl"),
            session("pi", "pi-old", 24 * 20, "/code/other"),
        ]
    }

    fn browser() -> Browser {
        Browser::new(fixture(), NOW, Some("/code/acryl".to_string()))
    }

    #[test]
    fn every_session_is_visible_before_any_filter() {
        let b = browser();
        assert_eq!(b.visible_len(), 3);
        assert_eq!(b.total(), 3);
        assert_eq!(b.selected().map(|s| s.id.as_str()), Some("claude-new"));
    }

    #[test]
    fn the_agent_filter_cycles_and_returns_to_all() {
        let mut b = browser();
        assert_eq!(b.agent_filter(), None);
        b.cycle_agent();
        assert_eq!(b.agent_filter(), Some("claude"));
        assert_eq!(b.visible_len(), 1);
        b.cycle_agent();
        assert_eq!(b.agent_filter(), Some("codex"));
        b.cycle_agent();
        assert_eq!(b.agent_filter(), Some("pi"));
        b.cycle_agent();
        assert_eq!(b.agent_filter(), None, "the cycle must come back to all");
        assert_eq!(b.visible_len(), 3);
    }

    #[test]
    fn recency_windows_narrow_the_list() {
        let mut b = browser();
        b.cycle_recency(); // last 24h
        assert_eq!(b.recency(), Recency::Day);
        assert_eq!(b.visible_len(), 1, "only the 1h-old session");
        b.cycle_recency(); // last 7d
        assert_eq!(b.visible_len(), 2, "the 50h-old one joins");
        b.cycle_recency(); // last 30d
        assert_eq!(b.visible_len(), 3);
        b.cycle_recency(); // back to any
        assert_eq!(b.recency(), Recency::Any);
    }

    #[test]
    fn the_project_filter_keeps_only_this_directory() {
        let mut b = browser();
        b.toggle_project();
        assert!(b.this_project());
        assert_eq!(b.visible_len(), 2);
        assert!(b.visible().all(|s| s.cwd.as_deref() == Some("/code/acryl")));
        b.toggle_project();
        assert_eq!(b.visible_len(), 3);
    }

    #[test]
    fn an_exact_id_is_ranked_above_fuzzy_matches() {
        let mut b = browser();
        // "pi" appears in the pi session's id and in no other, but "codex"
        // also sorts above alphabetical noise.
        b.set_query("pi-old");
        assert_eq!(
            b.selected().map(|s| s.id.as_str()),
            Some("pi-old"),
            "a pasted id must win outright"
        );
        assert_eq!(b.visible_len(), 1);
    }

    #[test]
    fn a_fuzzy_query_matches_on_words_not_just_ids() {
        let mut b = browser();
        b.set_query("other");
        assert_eq!(b.visible_len(), 1);
        assert_eq!(b.selected().map(|s| s.id.as_str()), Some("pi-old"));
    }

    #[test]
    fn a_query_that_matches_nothing_empties_the_list_without_panicking() {
        let mut b = browser();
        b.set_query("zzzzzz");
        assert_eq!(b.visible_len(), 0);
        assert!(b.selected().is_none());
        b.move_by(1);
        b.move_by(-1);
        assert!(b.selected().is_none());
    }

    #[test]
    fn the_cursor_stays_on_the_same_session_when_filters_change() {
        let mut b = browser();
        b.move_by(1);
        let id = b.selected().map(|s| s.id.clone());
        assert_eq!(id.as_deref(), Some("codex-mid"));
        // Adding a query that still matches keeps the cursor where it was.
        b.set_query("mid");
        assert_eq!(b.selected().map(|s| s.id.clone()), id);
    }

    #[test]
    fn movement_is_clamped_at_both_ends() {
        let mut b = browser();
        b.move_by(-5);
        assert_eq!(b.selected_row(), 0);
        b.move_by(99);
        assert_eq!(b.selected_row(), 2);
    }

    #[test]
    fn editing_the_query_refilters_as_you_type() {
        let mut b = browser();
        for c in "other".chars() {
            b.push_query(c);
        }
        assert_eq!(b.visible_len(), 1);
        for _ in 0..4 {
            b.pop_query();
        }
        assert_eq!(b.visible_len(), 3);
    }

    #[test]
    fn the_summary_names_every_active_filter() {
        let mut b = browser();
        assert_eq!(b.filter_summary(), "all agents · any time");
        b.cycle_agent();
        b.cycle_recency();
        b.toggle_project();
        b.set_query("x");
        assert_eq!(b.filter_summary(), "claude · last 24h · this project · “x”");
    }
}
