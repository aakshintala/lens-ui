//! Elicitation card discriminator (probe 6 composition).

use gpui::{AppContext as _, Entity, IntoElement, ParentElement, Styled, Window, div};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    text::TextView,
};

use crate::ask_user_question::AskUserQuestionForm;
use crate::fixtures::Fixture;
use crate::raw_editor::RawKeyValueEditor;
use crate::schema::parse_requested_schema;
use crate::schema_form::SchemaForm;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CardKind {
    Url,
    PlanReview,
    AskUserQuestion,
    Binary,
    SchemaForm,
    RawFallback,
}

pub struct ElicitationCard {
    pub kind: CardKind,
    pub schema_form: Option<Entity<SchemaForm>>,
    pub raw_editor: Option<Entity<RawKeyValueEditor>>,
    pub ask_form: Option<Entity<AskUserQuestionForm>>,
    pub fixture: Fixture,
}

impl ElicitationCard {
    pub fn from_fixture(fixture: Fixture, window: &mut Window, cx: &mut gpui::App) -> Self {
        if fixture.mode.as_deref() == Some("url") {
            return Self {
                kind: CardKind::Url,
                schema_form: None,
                raw_editor: None,
                ask_form: None,
                fixture,
            };
        }

        if fixture
            .structured
            .get("plan")
            .and_then(|p| p.as_str())
            .is_some()
        {
            return Self {
                kind: CardKind::PlanReview,
                schema_form: None,
                raw_editor: None,
                ask_form: None,
                fixture,
            };
        }

        if fixture.structured.get("questions").is_some() {
            let ask = AskUserQuestionForm::new_entity(&fixture.structured, window, cx);
            return Self {
                kind: CardKind::AskUserQuestion,
                schema_form: None,
                raw_editor: None,
                ask_form: ask,
                fixture,
            };
        }

        let parsed = parse_requested_schema(&fixture.requested_schema);
        if parsed.use_fallback {
            let editor = cx.new(|cx| RawKeyValueEditor::from_schema(&parsed, window, cx));
            return Self {
                kind: CardKind::RawFallback,
                schema_form: None,
                raw_editor: Some(editor),
                ask_form: None,
                fixture,
            };
        }

        let props = fixture
            .requested_schema
            .get("properties")
            .and_then(|p| p.as_object());
        let is_binary_only = props
            .map(|p| {
                p.len() <= 1
                    && p.contains_key("approve")
                    && fixture.requested_schema.get("required").is_none()
            })
            .unwrap_or(false)
            || fixture
                .requested_schema
                .as_object()
                .is_some_and(|o| o.is_empty());

        if is_binary_only && fixture.structured.is_null() {
            return Self {
                kind: CardKind::Binary,
                schema_form: None,
                raw_editor: None,
                ask_form: None,
                fixture,
            };
        }

        let form = cx.new(|cx| SchemaForm::from_schema(&parsed, window, cx));
        Self {
            kind: CardKind::SchemaForm,
            schema_form: Some(form),
            raw_editor: None,
            ask_form: None,
            fixture,
        }
    }

    pub fn render(&mut self, window: &mut Window, cx: &mut gpui::App) -> gpui::AnyElement {
        match self.kind {
            CardKind::Url => div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_sm().child("URL mode approval"))
                .child(
                    div()
                        .text_sm()
                        .text_color(gpui::rgb(0x60a5fa))
                        .child(self.fixture.url.clone().unwrap_or_default()),
                )
                .child(Button::new("open-url").label("Open / Approve").primary())
                .into_any_element(),
            CardKind::PlanReview => {
                let plan = self
                    .fixture
                    .structured
                    .get("plan")
                    .and_then(|p| p.as_str())
                    .unwrap_or("")
                    .to_string();
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_sm().child("Exit plan mode"))
                    .child(TextView::markdown("plan", plan, window, cx))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(Button::new("approve-plan").label("Approve").primary())
                            .child(Button::new("reject-plan").label("Reject").danger()),
                    )
                    .into_any_element()
            }
            CardKind::AskUserQuestion => {
                if let Some(form) = &self.ask_form {
                    form.update(cx, |f, cx| f.render(form, window, cx).into_any_element())
                } else {
                    div()
                        .child("AskUserQuestion parse failed")
                        .into_any_element()
                }
            }
            CardKind::Binary => div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_sm().child("Binary approve / deny"))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(Button::new("approve").label("Approve").primary())
                        .child(Button::new("deny").label("Deny").danger()),
                )
                .into_any_element(),
            CardKind::SchemaForm => {
                if let Some(form) = &self.schema_form {
                    form.update(cx, |f, cx| f.render(form, window, cx).into_any_element())
                } else {
                    div().child("missing form").into_any_element()
                }
            }
            CardKind::RawFallback => {
                if let Some(editor) = &self.raw_editor {
                    editor.update(cx, |e, cx| e.render(window, cx).into_any_element())
                } else {
                    div().child("missing raw editor").into_any_element()
                }
            }
        }
    }
}

pub fn card_kind_for_fixture(fixture: &Fixture) -> CardKind {
    if fixture.mode.as_deref() == Some("url") {
        return CardKind::Url;
    }
    if fixture.structured.get("plan").is_some() {
        return CardKind::PlanReview;
    }
    if fixture.structured.get("questions").is_some() {
        return CardKind::AskUserQuestion;
    }
    let parsed = parse_requested_schema(&fixture.requested_schema);
    if parsed.use_fallback {
        return CardKind::RawFallback;
    }
    let props = fixture
        .requested_schema
        .get("properties")
        .and_then(|p| p.as_object());
    let is_binary_only = props
        .map(|p| p.len() <= 1 && p.contains_key("approve"))
        .unwrap_or(false)
        || fixture
            .requested_schema
            .as_object()
            .is_some_and(|o| o.is_empty());
    if is_binary_only && fixture.structured.is_null() {
        CardKind::Binary
    } else {
        CardKind::SchemaForm
    }
}
