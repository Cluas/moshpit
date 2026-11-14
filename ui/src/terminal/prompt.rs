//! A question with a line of text for its answer (a name for a new window,
//! a new name for a folder), which the host asks with the platform's own
//! alert: NSH's `AlertDialog` has no text field (UIKit's
//! `UIAlertController.addTextField`, Android's `AlertDialog` with an
//! `EditText`), so the screen says what to ask and the host answers.
//!
//! The screen tells the host `{"ev": "prompt", "id": n, "title", "message",
//! "placeholder", "text", "confirm", "cancel"}` (every word already in the
//! app's language), and the host answers with the screen's own message
//! `{"op": "prompted", "id": n, "text": "…" | null}` (null: cancelled).

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use serde::Deserialize;
use serde_json::json;

/// What to ask.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Prompt {
    pub title: String,
    pub message: Option<String>,
    /// What the empty field says.
    pub placeholder: String,
    /// What the field starts with.
    pub text: String,
    /// The button that answers, and the one that does not.
    pub confirm: String,
    pub cancel: String,
}

/// The host's answer: `{"op": "prompted", "id": n, "text": … | null}`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub id: u32,
    #[serde(default)]
    pub text: Option<String>,
}

type Then = Box<dyn FnOnce(Option<String>)>;

/// The prompts a screen has asked and not heard back from, by id.
#[derive(Clone, Default)]
pub struct Prompts {
    inner: Rc<RefCell<(u32, HashMap<u32, Then>)>>,
}

impl Prompts {
    /// Asks the host `prompt` through `tell`; `then` hears the answer (none
    /// when the person cancelled).
    pub fn ask(
        &self,
        tell: &dyn Fn(serde_json::Value),
        prompt: Prompt,
        then: impl FnOnce(Option<String>) + 'static,
    ) {
        let id = {
            let mut inner = self.inner.borrow_mut();
            inner.0 += 1;
            let id = inner.0;
            inner.1.insert(id, Box::new(then));
            id
        };
        tell(event(id, &prompt));
    }

    /// The host answered prompt `answer.id`.
    pub fn answer(&self, answer: Answer) {
        let then = self.inner.borrow_mut().1.remove(&answer.id);
        match then {
            Some(then) => then(answer.text),
            None => log::warn!(target: "moshpit", "prompt {} answered twice", answer.id),
        }
    }

    /// How many are waiting.
    pub fn waiting(&self) -> usize {
        self.inner.borrow().1.len()
    }
}

/// The event that asks `prompt`.
pub fn event(id: u32, prompt: &Prompt) -> serde_json::Value {
    json!({
        "ev": "prompt",
        "id": id,
        "title": prompt.title,
        "message": prompt.message,
        "placeholder": prompt.placeholder,
        "text": prompt.text,
        "confirm": prompt.confirm,
        "cancel": prompt.cancel,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn an_answer_reaches_the_prompt_it_answers_once() {
        let prompts = Prompts::default();
        let told = Rc::new(RefCell::new(Vec::new()));
        let heard = Rc::new(RefCell::new(Vec::new()));
        for name in ["a", "b"] {
            let told = told.clone();
            let heard = heard.clone();
            prompts.ask(
                &move |event| told.borrow_mut().push(event),
                Prompt {
                    title: name.into(),
                    ..Prompt::default()
                },
                move |text| heard.borrow_mut().push((name, text)),
            );
        }
        assert_eq!(told.borrow()[0]["ev"], "prompt");
        assert_eq!(told.borrow()[1]["id"], 2);
        assert_eq!(told.borrow()[1]["message"], serde_json::Value::Null);
        assert_eq!(prompts.waiting(), 2);
        prompts.answer(Answer {
            id: 2,
            text: Some("x".into()),
        });
        prompts.answer(Answer { id: 1, text: None });
        let again = Cell::new(false);
        prompts.answer(Answer { id: 1, text: None });
        assert!(!again.get());
        assert_eq!(
            *heard.borrow(),
            vec![("b", Some("x".to_owned())), ("a", None)]
        );
        assert_eq!(prompts.waiting(), 0);
    }
}
