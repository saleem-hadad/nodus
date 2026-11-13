//! Forms of questions Claude asks the user, and the answers they send back.
//!
//! A form is `{"title", "submit"?, "questions": [...]}`. A question offers
//! choices (one, or several with `"multiple": true`) or asks for words
//! (`"type": "text"`). Every choice question also offers "Other / none of
//! these" last, and a note; a form of two questions or more has an
//! "Additional notes" box. The answers go back as one JSON object keyed by
//! question, so Claude can read them without guessing.

use serde_json::{Map, Value};

/// At most this many questions in a form.
pub const MAX_QUESTIONS: usize = 20;
/// At most this many choices in a question (one letter each, a to z).
pub const MAX_CHOICES: usize = 26;
/// The choice every choice question ends with.
pub const OTHER: &str = "Other / none of these";
/// The answer for a question left blank.
pub const SKIPPED: &str = "skipped";
/// The key the form-wide notes go under.
pub const NOTES: &str = "Additional notes";
/// The Send button's label when the form names none.
pub const DEFAULT_SUBMIT: &str = "Send answers";

#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub title: String,
    /// The Send button's label.
    pub submit: Option<String>,
    pub questions: Vec<Question>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    /// A short tag shown in a pill, like "Q1".
    pub tag: Option<String>,
    pub title: String,
    pub detail: Option<String>,
    /// The answer's key; the title when there's none.
    pub name: Option<String>,
    /// Several choices may be picked.
    pub multiple: bool,
    /// Answered in words, not by choosing.
    pub text: bool,
    pub placeholder: Option<String>,
    pub choices: Vec<Choice>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub label: String,
    pub detail: Option<String>,
    /// What the answer says; the label when there's none.
    pub value: Option<String>,
    pub recommended: bool,
}

impl Question {
    /// The key its answer goes under: its name, else its title.
    pub fn key(&self) -> &str {
        self.name.as_deref().filter(|n| !n.is_empty()).unwrap_or(&self.title)
    }

    /// The key its note goes under.
    pub fn note_key(&self) -> String {
        format!("{} (note)", self.key())
    }

    /// The choices as shown: its own, then "Other / none of these". A text
    /// question has none.
    pub fn shown_choices(&self) -> Vec<Choice> {
        if self.text {
            return Vec::new();
        }
        let mut out = self.choices.clone();
        out.push(Choice {
            label: OTHER.into(),
            detail: None,
            value: None,
            recommended: false,
        });
        out
    }
}

impl Choice {
    /// What picking it answers: its value, else its label.
    pub fn value(&self) -> &str {
        self.value.as_deref().unwrap_or(&self.label)
    }
}

impl Form {
    /// The Send button's label.
    pub fn submit_label(&self) -> &str {
        self.submit
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(DEFAULT_SUBMIT)
    }

    /// Whether it has the form-wide notes box.
    pub fn has_notes(&self) -> bool {
        self.questions.len() >= 2
    }
}

/// The letter a choice is shown with: a, b, c….
pub fn letter(i: usize) -> char {
    (b'a' + (i % 26) as u8) as char
}

fn fail<T>(path: &str, message: impl std::fmt::Display) -> Result<T, String> {
    Err(if path.is_empty() {
        message.to_string()
    } else {
        format!("{path}: {message}")
    })
}

fn string(obj: &Map<String, Value>, key: &str, path: &str) -> Result<Option<String>, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(Value::Number(n)) if key == "value" => Ok(Some(n.to_string())),
        Some(other) => fail(&format!("{path}.{key}"), format!("must be a string, not {other}")),
    }
}

/// Reads a form. Errors say what is wrong and where
/// (`questions[2].choices: …`), for Claude to fix.
pub fn parse_form(spec: &Value) -> Result<Form, String> {
    let Some(obj) = spec.as_object() else {
        return fail("", "a form is an object: {\"title\": \"…\", \"questions\": [...]}");
    };
    for key in obj.keys() {
        if !["title", "submit", "questions"].contains(&key.as_str()) {
            return fail(key, "isn't a form key (use title, submit and questions)");
        }
    }
    let title = match string(obj, "title", "")? {
        Some(t) if !t.trim().is_empty() => t,
        _ => return fail("title", "is missing: say what the questions are about"),
    };
    let submit = string(obj, "submit", "")?;
    let Some(items) = obj.get("questions").and_then(Value::as_array) else {
        return fail("questions", "is missing: give an array of questions");
    };
    if items.is_empty() {
        return fail("questions", "is empty: ask at least one question");
    }
    if items.len() > MAX_QUESTIONS {
        return fail(
            "questions",
            format!("has {}; a form asks at most {MAX_QUESTIONS}", items.len()),
        );
    }
    let mut questions: Vec<Question> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let path = format!("questions[{i}]");
        let q = parse_question(item, &path)?;
        if let Some(j) = questions.iter().position(|o| o.key() == q.key()) {
            return fail(
                &path,
                format!("its key \"{}\" is also questions[{j}]'s: give one a \"name\"", q.key()),
            );
        }
        if q.key() == NOTES {
            return fail(
                &path,
                format!("\"{NOTES}\" is kept for the form's notes: give it another \"name\""),
            );
        }
        questions.push(q);
    }
    Ok(Form {
        title,
        submit,
        questions,
    })
}

fn parse_question(v: &Value, path: &str) -> Result<Question, String> {
    let Some(obj) = v.as_object() else {
        return fail(
            path,
            "a question is an object with \"title\" and \"choices\" (or \"type\": \"text\")",
        );
    };
    for key in obj.keys() {
        if ![
            "tag",
            "title",
            "detail",
            "name",
            "multiple",
            "type",
            "placeholder",
            "choices",
        ]
        .contains(&key.as_str())
        {
            return fail(
                &format!("{path}.{key}"),
                "isn't a question key (use tag, title, detail, name, multiple, type, placeholder and choices)",
            );
        }
    }
    let title = match string(obj, "title", path)? {
        Some(t) if !t.trim().is_empty() => t,
        _ => return fail(&format!("{path}.title"), "is missing: ask the question"),
    };
    let text = match obj.get("type") {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) if s == "text" => true,
        Some(Value::String(s)) if s == "choice" => false,
        Some(other) => {
            return fail(
                &format!("{path}.type"),
                format!("{other} isn't a question type (use \"text\", or leave it out for choices)"),
            );
        }
    };
    let multiple = match obj.get("multiple") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(other) => {
            return fail(
                &format!("{path}.multiple"),
                format!("must be true or false, not {other}"),
            );
        }
    };
    let mut choices = Vec::new();
    if !text {
        let at = format!("{path}.choices");
        let items = match obj.get("choices") {
            Some(Value::Array(items)) => items.as_slice(),
            None | Some(Value::Null) => &[],
            Some(_) => return fail(&at, "must be an array of choices"),
        };
        if items.len() < 2 {
            return fail(
                &at,
                "needs at least 2 choices (or make it \"type\": \"text\" for a written answer)",
            );
        }
        if items.len() > MAX_CHOICES {
            return fail(
                &at,
                format!("has {}; a question offers at most {MAX_CHOICES}", items.len()),
            );
        }
        for (i, item) in items.iter().enumerate() {
            let cat = format!("{at}[{i}]");
            let choice = match item {
                Value::String(s) => Choice {
                    label: s.clone(),
                    detail: None,
                    value: None,
                    recommended: false,
                },
                Value::Object(o) => {
                    for key in o.keys() {
                        if !["label", "detail", "value", "recommended"].contains(&key.as_str()) {
                            return fail(
                                &format!("{cat}.{key}"),
                                "isn't a choice key (use label, detail, value and recommended)",
                            );
                        }
                    }
                    let Some(label) = string(o, "label", &cat)?.filter(|l| !l.trim().is_empty()) else {
                        return fail(&format!("{cat}.label"), "is missing");
                    };
                    let recommended = match o.get("recommended") {
                        None | Some(Value::Null) => false,
                        Some(Value::Bool(b)) => *b,
                        Some(other) => {
                            return fail(
                                &format!("{cat}.recommended"),
                                format!("must be true or false, not {other}"),
                            );
                        }
                    };
                    Choice {
                        label,
                        detail: string(o, "detail", &cat)?,
                        value: string(o, "value", &cat)?,
                        recommended,
                    }
                }
                other => {
                    return fail(
                        &cat,
                        format!("a choice is a string or {{\"label\": \"…\"}}, not {other}"),
                    );
                }
            };
            if choice.value() == OTHER {
                return fail(&cat, format!("\"{OTHER}\" is added to every question already"));
            }
            if let Some(j) = choices.iter().position(|c: &Choice| c.value() == choice.value()) {
                return fail(&cat, format!("\"{}\" is the same answer as {at}[{j}]", choice.value()));
            }
            choices.push(choice);
        }
    }
    Ok(Question {
        tag: string(obj, "tag", path)?,
        title,
        detail: string(obj, "detail", path)?,
        name: string(obj, "name", path)?,
        multiple,
        text,
        placeholder: string(obj, "placeholder", path)?,
        choices,
    })
}

/// What the user has picked and written so far, question by question.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FormState {
    /// Indices into each question's shown choices (its own, then Other).
    pub picks: Vec<Vec<usize>>,
    /// A text question's answer.
    pub texts: Vec<String>,
    /// Each choice question's note.
    pub notes: Vec<String>,
    /// The form-wide notes.
    pub extra: String,
}

impl FormState {
    /// Nothing picked or written.
    pub fn new(form: &Form) -> Self {
        let n = form.questions.len();
        Self {
            picks: vec![Vec::new(); n],
            texts: vec![String::new(); n],
            notes: vec![String::new(); n],
            extra: String::new(),
        }
    }

    /// Picks a choice, or clears it when picked already. A single-answer
    /// question drops its other pick.
    pub fn toggle(&mut self, form: &Form, question: usize, choice: usize) {
        let Some(q) = form.questions.get(question) else { return };
        let Some(picks) = self.picks.get_mut(question) else {
            return;
        };
        if let Some(at) = picks.iter().position(|&c| c == choice) {
            picks.remove(at);
        } else if q.multiple {
            picks.push(choice);
            picks.sort_unstable();
        } else {
            *picks = vec![choice];
        }
    }

    pub fn picked(&self, question: usize, choice: usize) -> bool {
        self.picks.get(question).is_some_and(|p| p.contains(&choice))
    }

    /// The state the sent answers describe, to show a form already answered.
    pub fn from_answers(form: &Form, answers: &Value) -> Self {
        let mut state = Self::new(form);
        let Some(obj) = answers.as_object() else { return state };
        for (i, q) in form.questions.iter().enumerate() {
            let choices = q.shown_choices();
            let find = |v: &str| choices.iter().position(|c| c.value() == v);
            match obj.get(q.key()) {
                Some(Value::String(s)) if q.text => {
                    if s != SKIPPED {
                        state.texts[i] = s.clone();
                    }
                }
                Some(Value::String(s)) => state.picks[i] = find(s).into_iter().collect(),
                Some(Value::Array(items)) => {
                    state.picks[i] = items.iter().filter_map(Value::as_str).filter_map(find).collect();
                }
                _ => {}
            }
            if let Some(Value::String(note)) = obj.get(&q.note_key()) {
                state.notes[i] = note.clone();
            }
        }
        if let Some(Value::String(extra)) = obj.get(NOTES) {
            state.extra = extra.clone();
        }
        state
    }
}

/// The answers as they go to Claude: an object keyed by question. A pick
/// is its value, several picks an array, words a string, and a question
/// left blank `"skipped"`. A note follows its question as `"<key> (note)"`,
/// and the form's notes come last as `"Additional notes"`.
pub fn answers_json(form: &Form, state: &FormState) -> Value {
    let mut out = Map::new();
    for (i, q) in form.questions.iter().enumerate() {
        let answer = if q.text {
            let text = state.texts.get(i).map_or("", |t| t.trim());
            if text.is_empty() {
                Value::from(SKIPPED)
            } else {
                Value::from(text)
            }
        } else {
            let choices = q.shown_choices();
            let picked: Vec<Value> = state
                .picks
                .get(i)
                .map(|p| {
                    let mut p = p.clone();
                    p.sort_unstable();
                    p.dedup();
                    p.iter()
                        .filter_map(|&c| choices.get(c))
                        .map(|c| Value::from(c.value()))
                        .collect()
                })
                .unwrap_or_default();
            match (picked.len(), q.multiple) {
                (0, _) => Value::from(SKIPPED),
                (_, true) => Value::Array(picked),
                (_, false) => picked.into_iter().next().unwrap_or(Value::Null),
            }
        };
        out.insert(q.key().to_owned(), answer);
        let note = state.notes.get(i).map_or("", |n| n.trim());
        if !q.text && !note.is_empty() {
            out.insert(q.note_key(), Value::from(note));
        }
    }
    let extra = state.extra.trim();
    if form.has_notes() && !extra.is_empty() {
        out.insert(NOTES.to_owned(), Value::from(extra));
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn form() -> Form {
        parse_form(&json!({
            "title": "Layout",
            "submit": "Go",
            "questions": [
                {"tag": "Q1", "title": "Which layout?", "name": "layout", "choices": [
                    {"label": "Tabs", "detail": "One at a time", "recommended": true},
                    {"label": "Split", "value": "split-view"}
                ]},
                {"title": "Which platforms?", "multiple": true, "choices": ["macOS", "Windows", "Linux"]},
                {"title": "Anything else?", "type": "text", "placeholder": "Say more"}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn reads_a_form() {
        let f = form();
        assert_eq!(f.title, "Layout");
        assert_eq!(f.submit_label(), "Go");
        assert_eq!(f.questions[0].key(), "layout");
        assert_eq!(f.questions[1].key(), "Which platforms?");
        assert_eq!(f.questions[0].choices[0].value(), "Tabs");
        assert_eq!(f.questions[0].choices[1].value(), "split-view");
        assert!(f.questions[0].choices[0].recommended);
        assert!(f.questions[2].text);
        assert_eq!(f.questions[0].shown_choices().last().unwrap().label, OTHER);
        assert!(f.questions[2].shown_choices().is_empty());
        assert!(f.has_notes());
        assert_eq!(letter(0), 'a');
        assert_eq!(letter(25), 'z');
    }

    #[test]
    fn errors_say_where() {
        let err = |v: Value| parse_form(&v).expect_err("an error");
        assert_eq!(
            err(json!({"questions": []})),
            "title: is missing: say what the questions are about"
        );
        assert_eq!(
            err(json!({"title": "t", "questions": []})),
            "questions: is empty: ask at least one question"
        );
        assert_eq!(
            err(json!({"title": "t", "questions": [{"title": "q", "choices": ["only"]}]})),
            "questions[0].choices: needs at least 2 choices (or make it \"type\": \"text\" for a written answer)"
        );
        assert_eq!(
            err(json!({"title": "t", "questions": [{"title": "q", "choices": ["a", "a"]}]})),
            "questions[0].choices[1]: \"a\" is the same answer as questions[0].choices[0]"
        );
        assert_eq!(
            err(json!({"title": "t", "questions": [{"choices": ["a", "b"]}]})),
            "questions[0].title: is missing: ask the question"
        );
        assert_eq!(
            err(json!({"title": "t", "questions": [{"title": "q", "type": "text"}, {"title": "q", "type": "text"}]})),
            "questions[1]: its key \"q\" is also questions[0]'s: give one a \"name\""
        );
        let many: Vec<Value> = (0..21)
            .map(|i| json!({"title": format!("q{i}"), "type": "text"}))
            .collect();
        assert_eq!(
            err(json!({"title": "t", "questions": many})),
            "questions: has 21; a form asks at most 20"
        );
        let letters: Vec<String> = (0..27).map(|i| format!("c{i}")).collect();
        assert_eq!(
            err(json!({"title": "t", "questions": [{"title": "q", "choices": letters}]})),
            "questions[0].choices: has 27; a question offers at most 26"
        );
        assert_eq!(
            err(json!({"title": "t", "questions": [{"title": "q", "kind": "x", "type": "text"}]})),
            "questions[0].kind: isn't a question key (use tag, title, detail, name, multiple, type, placeholder and choices)"
        );
    }

    #[test]
    fn picking_toggles() {
        let f = form();
        let mut s = FormState::new(&f);
        s.toggle(&f, 0, 0);
        s.toggle(&f, 0, 1);
        assert_eq!(s.picks[0], [1]);
        s.toggle(&f, 0, 1);
        assert!(s.picks[0].is_empty());
        s.toggle(&f, 1, 2);
        s.toggle(&f, 1, 0);
        assert_eq!(s.picks[1], [0, 2]);
        s.toggle(&f, 1, 2);
        assert_eq!(s.picks[1], [0]);
    }

    #[test]
    fn answers_are_keyed_by_question() {
        let f = form();
        let mut s = FormState::new(&f);
        assert_eq!(
            answers_json(&f, &s),
            json!({"layout": "skipped", "Which platforms?": "skipped", "Anything else?": "skipped"})
        );
        s.toggle(&f, 0, 1);
        s.notes[0] = " only on wide screens ".into();
        s.toggle(&f, 1, 2);
        s.toggle(&f, 1, 0);
        s.texts[2] = "Ship it".into();
        s.extra = "Thanks".into();
        let a = answers_json(&f, &s);
        assert_eq!(
            a,
            json!({
                "layout": "split-view",
                "layout (note)": "only on wide screens",
                "Which platforms?": ["macOS", "Linux"],
                "Anything else?": "Ship it",
                "Additional notes": "Thanks"
            })
        );
        // The keys keep the questions' order, each note after its question.
        let keys: Vec<&String> = a.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "layout",
                "layout (note)",
                "Which platforms?",
                "Anything else?",
                "Additional notes"
            ]
        );
        // "Other" answers with its label, beside the note that explains it.
        let mut s = FormState::new(&f);
        s.toggle(&f, 0, 2);
        s.notes[0] = "A sidebar".into();
        assert_eq!(answers_json(&f, &s)["layout"], json!(OTHER));
    }

    #[test]
    fn one_question_has_no_form_notes() {
        let f = parse_form(&json!({"title": "t", "questions": [{"title": "q", "choices": ["a", "b"]}]})).unwrap();
        assert!(!f.has_notes());
        assert_eq!(f.submit_label(), DEFAULT_SUBMIT);
        let mut s = FormState::new(&f);
        s.extra = "ignored".into();
        assert_eq!(answers_json(&f, &s), json!({"q": "skipped"}));
    }

    #[test]
    fn sent_answers_read_back() {
        let f = form();
        let mut s = FormState::new(&f);
        s.toggle(&f, 0, 1);
        s.toggle(&f, 1, 1);
        s.notes[1] = "mostly".into();
        s.texts[2] = "Nope".into();
        s.extra = "x".into();
        let sent = answers_json(&f, &s);
        assert_eq!(FormState::from_answers(&f, &sent), s);
        // Skipped questions come back blank.
        let blank = FormState::new(&f);
        assert_eq!(FormState::from_answers(&f, &answers_json(&f, &blank)), blank);
    }
}
