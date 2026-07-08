//! AskUserQuestion carousel — port of omnigent web form semantics.

use std::collections::HashSet;

use gpui::{
    div, prelude::*, App, AppContext as _, Entity, IntoElement, ParentElement, Styled, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    input::{Input, InputState},
    radio::Radio,
    Disableable,
};
use serde_json::{Map, Value};

#[derive(Clone, Debug)]
pub struct QuestionOption {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Question {
    pub id: Option<String>,
    pub question: String,
    pub header: Option<String>,
    pub multi_select: bool,
    pub options: Vec<QuestionOption>,
}

pub fn question_key(q: &Question) -> String {
    if let Some(id) = &q.id {
        if !id.is_empty() {
            return id.clone();
        }
    }
    q.question.clone()
}

pub struct QuestionUiState {
    pub key: String,
    pub single_selection: Option<String>,
    pub multi_selection: HashSet<String>,
    pub custom_selected: bool,
    pub custom_input: Entity<InputState>,
}

pub struct AskUserQuestionForm {
    pub questions: Vec<Question>,
    pub states: Vec<QuestionUiState>,
    pub current_index: usize,
}

impl AskUserQuestionForm {
    pub fn new_entity(
        payload: &Value,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Entity<Self>> {
        let questions_raw = payload.get("questions")?.as_array()?;
        let mut questions = Vec::new();
        for entry in questions_raw {
            let question = entry.get("question")?.as_str()?.to_string();
            let options_raw = entry.get("options")?.as_array()?;
            let mut options = Vec::new();
            for opt in options_raw {
                let label = opt.get("label")?.as_str()?.to_string();
                let description = opt
                    .get("description")
                    .and_then(|d| d.as_str())
                    .map(String::from);
                options.push(QuestionOption { label, description });
            }
            if options.is_empty() {
                continue;
            }
            questions.push(Question {
                id: entry
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                question,
                header: entry
                    .get("header")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                multi_select: entry.get("multiSelect") == Some(&Value::Bool(true)),
                options,
            });
        }
        if questions.is_empty() {
            return None;
        }

        Some(cx.new(|cx| {
            let states = questions
                .iter()
                .map(|q| {
                    let custom_input = cx.new(|cx| {
                        InputState::new(window, cx).placeholder("Type something")
                    });
                    QuestionUiState {
                        key: question_key(q),
                        single_selection: None,
                        multi_selection: HashSet::new(),
                        custom_selected: false,
                        custom_input,
                    }
                })
                .collect();

            Self {
                questions,
                states,
                current_index: 0,
            }
        }))
    }

    pub fn answer_for_question(&self, ix: usize, cx: &gpui::App) -> Option<Value> {
        let q = &self.questions[ix];
        let st = &self.states[ix];
        let custom_value = st.custom_input.read(cx).value().trim().to_string();

        if q.multi_select {
            let mut selected: Vec<String> = st.multi_selection.iter().cloned().collect();
            if st.custom_selected && !custom_value.is_empty() {
                if !selected.iter().any(|s| s == &custom_value) {
                    selected.push(custom_value);
                }
            }
            selected.sort();
            if selected.is_empty() {
                None
            } else {
                Some(Value::Array(
                    selected.into_iter().map(Value::String).collect(),
                ))
            }
        } else if st.custom_selected {
            if custom_value.is_empty() {
                None
            } else {
                Some(Value::String(custom_value))
            }
        } else {
            st.single_selection
                .as_ref()
                .filter(|s| !s.is_empty())
                .map(|s| Value::String(s.clone()))
        }
    }

    pub fn all_answered(&self, cx: &gpui::App) -> bool {
        (0..self.questions.len()).all(|ix| self.answer_for_question(ix, cx).is_some())
    }

    pub fn submit(&self, cx: &gpui::App) -> Result<Map<String, Value>, String> {
        if !self.all_answered(cx) {
            return Err("not all questions answered".into());
        }
        let mut map = Map::new();
        for ix in 0..self.questions.len() {
            let key = self.states[ix].key.clone();
            let answer = self.answer_for_question(ix, cx).unwrap();
            map.insert(key, answer);
        }
        Ok(map)
    }

    pub fn fill_probe_answers(
        &mut self,
        answers: &Map<String, Value>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) {
        for (ix, st) in self.states.iter_mut().enumerate() {
            let q = &self.questions[ix];
            let Some(val) = answers.get(&st.key) else {
                continue;
            };
            if q.multi_select {
                st.multi_selection.clear();
                st.custom_selected = false;
                if let Value::Array(arr) = val {
                    for item in arr {
                        if let Value::String(s) = item {
                            let known: HashSet<_> =
                                q.options.iter().map(|o| o.label.as_str()).collect();
                            if known.contains(s.as_str()) {
                                st.multi_selection.insert(s.clone());
                            } else {
                                st.custom_selected = true;
                                st.custom_input.update(cx, |input, cx| {
                                    input.set_value(s.clone(), window, cx);
                                });
                            }
                        }
                    }
                }
            } else if let Value::String(s) = val {
                let known: HashSet<_> = q.options.iter().map(|o| o.label.as_str()).collect();
                if known.contains(s.as_str()) {
                    st.single_selection = Some(s.clone());
                    st.custom_selected = false;
                } else {
                    st.custom_selected = true;
                    st.single_selection = None;
                    st.custom_input.update(cx, |input, cx| {
                        input.set_value(s.clone(), window, cx);
                    });
                }
            }
        }
    }

    pub fn on_custom_input_changed(&mut self, ix: usize, cx: &gpui::App) {
        let text = self.states[ix].custom_input.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let multi = self.questions[ix].multi_select;
        if multi {
            self.states[ix].custom_selected = true;
        } else {
            self.states[ix].custom_selected = true;
            self.states[ix].single_selection = None;
        }
    }

    pub fn render(
        &mut self,
        entity: &Entity<Self>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let entity = entity.clone();
        let ix = self.current_index;
        let q = self.questions[ix].clone();
        let total = self.questions.len();
        let all_answered = self.all_answered(cx);
        let is_first = ix == 0;
        let is_last = ix + 1 == total;

        let header_badge = q.header.clone().map(|h| {
            div()
                .text_xs()
                .px_1p5()
                .py_0p5()
                .rounded_md()
                .bg(gpui::rgb(0x333333))
                .child(h)
        });

        let option_rows = q.options.iter().enumerate().map(|(opt_ix, opt)| {
            let label = opt.label.clone();
            let desc = opt.description.clone();
            let ent = entity.clone();
            let opt_label = label.clone();

            if q.multi_select {
                let checked = self.states[ix].multi_selection.contains(&label);
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        Checkbox::new(("cb", ix * 100 + opt_ix))
                            .checked(checked)
                            .label(label.clone())
                            .on_click(move |new_checked, _, cx| {
                                ent.update(cx, |form, _| {
                                    let st = &mut form.states[ix];
                                    if *new_checked {
                                        st.multi_selection.insert(opt_label.clone());
                                    } else {
                                        st.multi_selection.remove(&opt_label);
                                    }
                                });
                            }),
                    )
                    .when_some(desc, |this, d| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(gpui::rgb(0x888888))
                                .ml_6()
                                .child(d),
                        )
                    })
            } else {
                let checked = self.states[ix].single_selection.as_deref() == Some(label.as_str())
                    && !self.states[ix].custom_selected;
                let opt_label2 = label.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        Radio::new(("radio", ix * 100 + opt_ix))
                            .checked(checked)
                            .label(label.clone())
                            .on_click(move |_, _, cx| {
                                ent.update(cx, |form, _| {
                                    let st = &mut form.states[ix];
                                    st.single_selection = Some(opt_label2.clone());
                                    st.custom_selected = false;
                                });
                            }),
                    )
                    .when_some(desc, |this, d| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(gpui::rgb(0x888888))
                                .ml_6()
                                .child(d),
                        )
                    })
            }
        });

        let custom_checked = self.states[ix].custom_selected;
        let custom_input = self.states[ix].custom_input.clone();
        let ent_custom = entity.clone();

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .text_xs()
                    .text_color(gpui::rgb(0x888888))
                    .child(format!("Question {} of {}:", ix + 1, total))
                    .children(header_badge),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(q.question.clone()),
            )
            .children(option_rows)
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(if q.multi_select {
                        Checkbox::new(("custom-cb", ix))
                            .checked(custom_checked)
                            .on_click({
                                let ent = ent_custom.clone();
                                move |new_checked, _, cx| {
                                    ent.update(cx, |form, _| {
                                        form.states[ix].custom_selected = *new_checked;
                                    });
                                }
                            })
                            .into_any_element()
                    } else {
                        Radio::new(("custom-radio", ix))
                            .checked(custom_checked)
                            .on_click({
                                let ent = ent_custom.clone();
                                move |_, _, cx| {
                                    ent.update(cx, |form, _| {
                                        let st = &mut form.states[ix];
                                        st.custom_selected = true;
                                        st.single_selection = None;
                                    });
                                }
                            })
                            .into_any_element()
                    })
                    .child(
                        Input::new(&custom_input),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("prev")
                            .label("Prev")
                            .disabled(is_first)
                            .on_click({
                                let ent = entity.clone();
                                move |_, _, cx| {
                                    ent.update(cx, |form, _| {
                                        form.current_index = form.current_index.saturating_sub(1);
                                    });
                                }
                            }),
                    )
                    .child(if !is_last {
                        Button::new("next")
                            .label("Next")
                            .on_click({
                                let ent = entity.clone();
                                move |_, _, cx| {
                                    ent.update(cx, |form, _| {
                                        form.current_index =
                                            (form.current_index + 1).min(form.questions.len() - 1);
                                    });
                                }
                            })
                            .into_any_element()
                    } else {
                        Button::new("submit-aq")
                            .label("Submit")
                            .primary()
                            .disabled(!all_answered)
                            .on_click({
                                let ent = entity.clone();
                                move |_, _, cx| {
                                    let _ = ent.read(cx).submit(cx);
                                }
                            })
                            .into_any_element()
                    }),
            )
    }
}
