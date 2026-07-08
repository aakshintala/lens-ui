//! Raw key/value editor fallback when schema has unsupported properties.

use std::collections::BTreeMap;

use gpui::{div, prelude::*, Entity, IntoElement, ParentElement, Styled, Window};
use gpui_component::input::{Input, InputState};

use crate::schema::ParsedSchema;

pub struct RawKeyValueEditor {
    pub entries: Vec<(String, Entity<InputState>)>,
}

impl RawKeyValueEditor {
    pub fn from_schema(
        schema: &ParsedSchema,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let entries = schema
            .fields
            .iter()
            .map(|f| {
                let state = cx.new(|cx| InputState::new(window, cx));
                if let Some(default) = &f.default_value {
                    let text = default.to_string();
                    state.update(cx, |s, cx| s.set_value(text, window, cx));
                }
                (f.key.clone(), state)
            })
            .collect();
        Self { entries }
    }

    pub fn read_back(&self, cx: &gpui::App) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for (key, state) in &self.entries {
            out.insert(key.clone(), state.read(cx).value().to_string());
        }
        out
    }

    pub fn render(&self, window: &mut Window, cx: &gpui::App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(self.entries.iter().map(|(key, state)| {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .child(format!("{key} (raw)")),
                    )
                    .child(Input::new(state))
            }))
            .child(
                div()
                    .text_xs()
                    .text_color(gpui::rgb(0xf59e0b))
                    .child("Fallback: unsupported schema shape — edit key/value pairs manually."),
            )
    }
}
