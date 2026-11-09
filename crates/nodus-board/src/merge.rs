//! The merge dialog's state ("New session from context", spec 11 MERGE):
//! each source session summarizes itself in a forked copy, then the user
//! reviews the combined handoff and starts a session on it. Headless, so
//! it's tested without a window; the dialog in nodus-ui drives it.
//!
//! Ports the logic of `src/renderer/src/components/board/MergeDialog.tsx`.

use std::collections::HashSet;

use indexmap::IndexMap;
use nodus_domain::{SummarizePhase, SummarizeProgress, SummaryResult, js};

/// What the new session is told when the instruction is left blank.
pub const DEFAULT_INSTRUCTION: &str =
    "Read the context above, confirm briefly what you understand, then wait for my instructions.";
/// A failed summary's error when the session answered with nothing.
pub const EMPTY_SUMMARY: &str = "The session returned an empty summary.";

/// `truncate` (layout.ts): lengths in UTF-16 units.
pub fn truncate(text: &str, max: usize) -> String {
    if js::utf16_len(text) <= max {
        return text.to_owned();
    }
    format!("{}…", js::trim_end(&js::slice_utf16(text, 0, max.saturating_sub(1))))
}

/// MERGE-003: trimmed, every whitespace run one space, empty → none.
pub fn normalize_focus(focus: Option<&str>) -> Option<String> {
    let f = js::collapse_spaces(js::trim(focus?));
    (!f.is_empty()).then_some(f)
}

/// The new session's default name (`defaultName`).
pub fn default_name(titles: &[String], focus: Option<&str>) -> String {
    if let Some(focus) = focus {
        return truncate(&format!("From search: {focus}"), 64);
    }
    let titles: Vec<String> = titles.iter().map(|t| truncate(t, 28)).collect();
    let joined = if titles.len() <= 2 {
        titles.join(" + ")
    } else {
        format!("{} + {} more", titles[..2].join(" + "), titles.len() - 2)
    };
    truncate(&format!("From: {joined}"), 64)
}

/// One session's part of the handoff (`section`).
pub fn section(index: usize, title: &str, summary: &str) -> String {
    format!("## Session {index} — {title}\n{}", js::trim(summary))
}

/// The first prompt: the context, a rule, then what to do (MERGE-015).
pub fn initial_prompt(context: &str, instruction: &str) -> String {
    let body = match js::trim(instruction) {
        "" => DEFAULT_INSTRUCTION,
        b => b,
    };
    match js::trim(context) {
        "" => body.to_owned(),
        c => format!("{c}\n\n---\n\n{body}"),
    }
}

/// Where a source has got to (`ItemState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub phase: SummarizePhase,
    pub title: String,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub skipped: bool,
}

/// The dialog's two steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Gathering,
    Review,
}

/// The dialog's state, minus its text fields' editing.
#[derive(Debug, Clone)]
pub struct Merge {
    /// The sources' ids, in the order given.
    pub order: Vec<String>,
    pub items: IndexMap<String, Item>,
    pub focus: Option<String>,
    pub step: Step,
    /// The Context field, and whether the user has edited it.
    pub context: String,
    pub dirty: bool,
    requests: HashSet<String>,
}

impl Merge {
    /// The dialog for these sources (`{ sessionId, title }`), every one queued.
    pub fn new(sources: &[(String, String)], focus: Option<&str>) -> Self {
        let items = sources
            .iter()
            .map(|(id, title)| {
                (
                    id.clone(),
                    Item {
                        phase: SummarizePhase::Queued,
                        title: title.clone(),
                        summary: None,
                        error: None,
                        skipped: false,
                    },
                )
            })
            .collect();
        Self {
            order: sources.iter().map(|(id, _)| id.clone()).collect(),
            items,
            focus: normalize_focus(focus),
            step: Step::Gathering,
            context: String::new(),
            dirty: false,
            requests: HashSet::new(),
        }
    }

    /// The sources in order with their state.
    pub fn list(&self) -> impl Iterator<Item = (&String, &Item)> {
        self.order.iter().filter_map(|id| self.items.get(id).map(|it| (id, it)))
    }

    /// The default name for the Name field.
    pub fn default_name(&self) -> String {
        let titles: Vec<String> = self.list().map(|(_, it)| it.title.clone()).collect();
        default_name(&titles, self.focus.as_deref())
    }

    /// MERGE-004, MERGE-009: a summarize request for `ids` under
    /// `request_id`; they go back to queued, unskipped, error cleared.
    pub fn begin(&mut self, ids: &[String], request_id: &str) {
        self.requests.insert(request_id.to_owned());
        for id in ids {
            if let Some(it) = self.items.get_mut(id) {
                it.phase = SummarizePhase::Queued;
                it.error = None;
                it.skipped = false;
            }
        }
    }

    /// MERGE-004: a progress event, for this dialog's requests only; a
    /// source already done never changes.
    pub fn progress(&mut self, p: &SummarizeProgress) -> bool {
        if !self.requests.contains(&p.request_id) {
            return false;
        }
        let Some(it) = self.items.get_mut(&p.session_id) else {
            return false;
        };
        if it.phase == SummarizePhase::Done {
            return false;
        }
        it.phase = p.phase;
        it.error = p.error.clone();
        true
    }

    /// MERGE-006, MERGE-012: the request answered (or threw, `Err(message)`).
    pub fn results(&mut self, ids: &[String], answer: Result<Vec<SummaryResult>, String>) {
        let results = answer.unwrap_or_else(|message| {
            ids.iter()
                .map(|id| SummaryResult {
                    session_id: id.clone(),
                    title: String::new(),
                    summary: String::new(),
                    error: Some(message.clone()),
                })
                .collect()
        });
        // Already edited: retried summaries are appended instead.
        if self.dirty {
            let added: Vec<&SummaryResult> = results
                .iter()
                .filter(|r| r.error.as_deref().is_none_or(str::is_empty) && !js::trim(&r.summary).is_empty())
                .collect();
            if !added.is_empty() {
                let base = self
                    .items
                    .values()
                    .filter(|it| it.phase == SummarizePhase::Done)
                    .count();
                let mut parts = vec![js::trim_end(&self.context).to_owned()];
                for (i, r) in added.iter().enumerate() {
                    let title = if r.title.is_empty() {
                        self.items
                            .get(&r.session_id)
                            .map(|it| it.title.clone())
                            .unwrap_or_default()
                    } else {
                        r.title.clone()
                    };
                    parts.push(section(base + i + 1, &title, &r.summary));
                }
                self.context = parts.join("\n\n");
            }
        }
        for r in &results {
            let Some(it) = self.items.get_mut(&r.session_id) else {
                continue;
            };
            let error = r.error.as_deref().filter(|e| !e.is_empty());
            if error.is_some() || js::trim(&r.summary).is_empty() {
                it.phase = SummarizePhase::Error;
                it.error = Some(error.unwrap_or(EMPTY_SUMMARY).to_owned());
            } else {
                it.phase = SummarizePhase::Done;
                it.summary = Some(r.summary.clone());
                if !r.title.is_empty() {
                    it.title = r.title.clone();
                }
                it.error = None;
            }
        }
        self.settle();
    }

    /// MERGE-010.
    pub fn skip(&mut self, id: &str) {
        if let Some(it) = self.items.get_mut(id) {
            it.skipped = true;
        }
        self.settle();
    }

    /// The user edited the Context field (MERGE-011).
    pub fn edit_context(&mut self, text: String) {
        self.context = text;
        self.dirty = true;
    }

    /// `Reset to generated` (MERGE-011).
    pub fn reset_context(&mut self) {
        self.dirty = false;
        self.settle();
    }

    /// Follows the generated context until edited, and moves to review once
    /// nothing is pending (MERGE-007, MERGE-011). Call after any change.
    pub fn settle(&mut self) {
        if self.step == Step::Gathering && self.pending() == 0 {
            self.step = Step::Review;
        }
        if !self.dirty {
            self.context = self.generated();
        }
    }

    pub fn pending(&self) -> usize {
        self.list()
            .filter(|(_, it)| matches!(it.phase, SummarizePhase::Queued | SummarizePhase::Running))
            .count()
    }

    /// Failed and not skipped: they keep the session from starting.
    pub fn blocking(&self) -> usize {
        self.list()
            .filter(|(_, it)| it.phase == SummarizePhase::Error && !it.skipped)
            .count()
    }

    /// The parents: done and not skipped, in source order.
    pub fn succeeded(&self) -> Vec<String> {
        self.list()
            .filter(|(_, it)| it.phase == SummarizePhase::Done && !it.skipped)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// The handoff as generated (`compose`).
    pub fn generated(&self) -> String {
        let ok: Vec<&Item> = self
            .list()
            .map(|(_, it)| it)
            .filter(|it| {
                it.phase == SummarizePhase::Done && it.summary.as_deref().is_some_and(|s| !s.is_empty()) && !it.skipped
            })
            .collect();
        if ok.is_empty() {
            return String::new();
        }
        let n = ok.len();
        let head = format!(
            "I'm continuing work from {n} earlier Claude Code session{} in this repository. Here is {}",
            if n == 1 { "" } else { "s" },
            match &self.focus {
                Some(focus) => format!("what {} did about \"{focus}\":", if n == 1 { "it" } else { "each" }),
                None => format!("a handoff summary of {}:", if n == 1 { "it" } else { "each" }),
            }
        );
        let mut parts = vec![head];
        for (i, it) in ok.iter().enumerate() {
            parts.push(section(i + 1, &it.title, it.summary.as_deref().unwrap_or("")));
        }
        parts.join("\n\n")
    }

    /// MERGE-014.
    pub fn can_start(&self, instruction: &str, starting: bool) -> bool {
        self.step == Step::Review
            && !starting
            && self.pending() == 0
            && self.blocking() == 0
            && (!js::trim(&self.context).is_empty() || !js::trim(instruction).is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources() -> Vec<(String, String)> {
        vec![("a".into(), "Fix cart".into()), ("b".into(), "Coupons".into())]
    }

    fn ok(id: &str, title: &str, summary: &str) -> SummaryResult {
        SummaryResult {
            session_id: id.into(),
            title: title.into(),
            summary: summary.into(),
            error: None,
        }
    }

    fn progress(request: &str, id: &str, phase: SummarizePhase) -> SummarizeProgress {
        SummarizeProgress {
            request_id: request.into(),
            session_id: id.into(),
            phase,
            error: None,
        }
    }

    #[test]
    fn names_the_session_after_its_sources_or_the_search() {
        let t = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(default_name(&t(&["A", "B"]), None), "From: A + B");
        assert_eq!(default_name(&t(&["A", "B", "C", "D"]), None), "From: A + B + 2 more");
        assert_eq!(
            default_name(&t(&["An extremely long session title here", "B"]), None),
            "From: An extremely long session t… + B"
        );
        assert_eq!(default_name(&t(&["A"]), Some("coupon")), "From search: coupon");
    }

    #[test]
    fn normalizes_the_focus() {
        assert_eq!(normalize_focus(Some("  a \n  b ")), Some("a b".into()));
        assert_eq!(normalize_focus(Some("   ")), None);
        assert_eq!(normalize_focus(None), None);
    }

    #[test]
    fn writes_the_first_prompt() {
        assert_eq!(
            initial_prompt("  ctx ", ""),
            format!("ctx\n\n---\n\n{DEFAULT_INSTRUCTION}")
        );
        assert_eq!(initial_prompt("", " do it "), "do it");
    }

    #[test]
    fn gathers_then_reviews_once_nothing_is_pending() {
        let mut m = Merge::new(&sources(), None);
        let ids = m.order.clone();
        m.begin(&ids, "r1");
        assert!(m.progress(&progress("r1", "a", SummarizePhase::Running)));
        // Another dialog's request is ignored.
        assert!(!m.progress(&progress("r2", "b", SummarizePhase::Running)));
        m.settle();
        assert_eq!(m.step, Step::Gathering);
        m.results(&ids, Ok(vec![ok("a", "Cart store", " Moved it. "), ok("b", "", "")]));
        assert_eq!(m.step, Step::Review);
        assert_eq!(m.items["a"].title, "Cart store");
        assert_eq!(m.items["b"].phase, SummarizePhase::Error);
        assert_eq!(m.items["b"].error.as_deref(), Some(EMPTY_SUMMARY));
        assert_eq!(
            m.context,
            "I'm continuing work from 1 earlier Claude Code session in this repository. Here is a handoff \
             summary of it:\n\n## Session 1 — Cart store\nMoved it."
        );
        assert!(!m.can_start("go", false));
        m.skip("b");
        assert!(m.can_start("", false));
        assert_eq!(m.succeeded(), vec!["a".to_owned()]);
        // A done source never goes back on a late event.
        assert!(!m.progress(&progress("r1", "a", SummarizePhase::Running)));
    }

    #[test]
    fn focus_shapes_the_head() {
        let mut m = Merge::new(&sources(), Some("coupon  codes"));
        let ids = m.order.clone();
        m.begin(&ids, "r");
        m.results(&ids, Ok(vec![ok("a", "", "x"), ok("b", "", "y")]));
        assert!(m.context.starts_with(
            "I'm continuing work from 2 earlier Claude Code sessions in this repository. Here is what each did \
             about \"coupon codes\":"
        ));
        assert!(m.context.ends_with("## Session 2 — Coupons\ny"));
    }

    #[test]
    fn a_thrown_request_fails_every_source_in_it() {
        let mut m = Merge::new(&sources(), None);
        let ids = m.order.clone();
        m.begin(&ids, "r");
        m.results(&ids, Err("no claude".into()));
        assert!(m.list().all(|(_, it)| it.error.as_deref() == Some("no claude")));
        assert_eq!(m.step, Step::Review);
        assert_eq!(m.blocking(), 2);
    }

    #[test]
    fn appends_a_retry_to_an_edited_context() {
        let mut m = Merge::new(&sources(), None);
        let ids = m.order.clone();
        m.begin(&ids, "r1");
        m.results(&ids, Ok(vec![ok("a", "", "one"), ok("b", "", "")]));
        m.edit_context("Mine.  ".into());
        m.begin(&["b".to_owned()], "r2");
        assert_eq!(m.items["b"].phase, SummarizePhase::Queued);
        m.results(&["b".to_owned()], Ok(vec![ok("b", "Coupon tests", "two")]));
        assert_eq!(m.context, "Mine.\n\n## Session 2 — Coupon tests\ntwo");
        m.reset_context();
        assert!(m.context.contains("## Session 2 — Coupon tests\ntwo"));
        assert!(m.context.contains("## Session 1 — Fix cart\none"));
    }
}
