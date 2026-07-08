//! Runtime schema → heterogeneous widget collection (the §4.3 crux).

use gpui::{
    div, prelude::*, Entity, IntoElement, ParentElement, SharedString, Styled, Window,
};
use gpui_component::{
    input::{Input, InputState, NumberInput},
    select::{SearchableVec, Select, SelectState},
    switch::Switch,
    IndexPath,
};
use serde_json::{Map, Value};

use crate::schema::{FieldKind, ParsedField, ParsedSchema};

#[derive(Debug, Clone)]
pub struct FieldError {
    pub key: String,
    pub message: String,
}

enum FieldWidget {
    String {
        input: Entity<InputState>,
    },
    Number {
        is_integer: bool,
        input: Entity<InputState>,
    },
    Bool {
        value: bool,
    },
    Enum {
        select: Entity<SelectState<SearchableVec<SharedString>>>,
    },
}

pub struct FieldState {
    pub key: String,
    pub kind: FieldKind,
    pub required: bool,
    widget: FieldWidget,
    pub inline_error: Option<String>,
}

pub struct SchemaForm {
    pub fields: Vec<FieldState>,
}

impl SchemaForm {
    pub fn from_schema(
        parsed: &ParsedSchema,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let fields = parsed
            .fields
            .iter()
            .map(|f| Self::build_field(f, window, cx))
            .collect();
        Self { fields }
    }

    fn build_field(
        f: &ParsedField,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> FieldState {
        let widget = match &f.kind {
            FieldKind::String => {
                let input = cx.new(|cx| InputState::new(window, cx));
                apply_default_string(&input, &f.default_value, window, cx);
                FieldWidget::String { input }
            }
            FieldKind::Number | FieldKind::Integer => {
                let input = cx.new(|cx| InputState::new(window, cx));
                apply_default_string(&input, &f.default_value, window, cx);
                FieldWidget::Number {
                    is_integer: f.kind == FieldKind::Integer,
                    input,
                }
            }
            FieldKind::Bool => FieldWidget::Bool {
                value: f
                    .default_value
                    .as_ref()
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            },
            FieldKind::Enum(variants) => {
                let items: Vec<SharedString> =
                    variants.iter().map(|s| SharedString::from(s.clone())).collect();
                let delegate = SearchableVec::new(items);
                let mut initial: Option<IndexPath> = None;
                if let Some(Value::String(s)) = &f.default_value {
                    if let Some(ix) = variants.iter().position(|v| v == s) {
                        initial = Some(IndexPath::default().row(ix));
                    }
                }
                let select = cx.new(|cx| SelectState::new(delegate, initial, window, cx));
                FieldWidget::Enum { select }
            }
            FieldKind::Unsupported => {
                let input = cx.new(|cx| InputState::new(window, cx));
                FieldWidget::String { input }
            }
        };

        FieldState {
            key: f.key.clone(),
            kind: f.kind.clone(),
            required: f.required,
            widget,
            inline_error: None,
        }
    }

    pub fn set_bool(&mut self, key: &str, value: bool) {
        if let Some(field) = self.fields.iter_mut().find(|f| f.key == key) {
            if let FieldWidget::Bool { value: ref mut v } = field.widget {
                *v = value;
            }
        }
    }

    pub fn set_string_value(
        &mut self,
        key: &str,
        value: &str,
        window: &mut Window,
        cx: &mut gpui::App,
    ) {
        let owned = value.to_string();
        if let Some(field) = self.fields.iter().find(|f| f.key == key) {
            match &field.widget {
                FieldWidget::String { input } | FieldWidget::Number { input, .. } => {
                    input.update(cx, |s, cx| s.set_value(owned.clone(), window, cx));
                }
                FieldWidget::Enum { select } => {
                    select.update(cx, |s, cx| {
                        s.set_selected_value(&SharedString::from(value.to_string()), window, cx);
                    });
                }
                _ => {}
            }
        }
    }

    pub fn clear_errors(&mut self) {
        for f in &mut self.fields {
            f.inline_error = None;
        }
    }

    pub fn validate(&mut self, cx: &gpui::App) -> bool {
        let mut ok = true;
        for field in &mut self.fields {
            field.inline_error = None;
            let empty = field_value_empty(field, cx);
            if field.required && empty {
                field.inline_error = Some("Required".into());
                ok = false;
                continue;
            }
            if let FieldWidget::Number { is_integer, input } = &field.widget {
                let text = input.read(cx).value();
                if text.is_empty() {
                    continue;
                }
                if *is_integer {
                    if text.parse::<i64>().is_err() {
                        field.inline_error = Some("Invalid integer".into());
                        ok = false;
                    }
                } else if text.parse::<f64>().is_err() {
                    field.inline_error = Some("Invalid number".into());
                    ok = false;
                }
            }
        }
        ok
    }

    pub fn can_submit(&mut self, cx: &gpui::App) -> bool {
        self.validate(cx)
    }

    pub fn submit(&mut self, cx: &gpui::App) -> Result<Map<String, Value>, Vec<FieldError>> {
        if !self.validate(cx) {
            return Err(
                self.fields
                    .iter()
                    .filter_map(|f| {
                        f.inline_error.as_ref().map(|msg| FieldError {
                            key: f.key.clone(),
                            message: msg.clone(),
                        })
                    })
                    .collect(),
            );
        }

        let mut map = Map::new();
        for field in &self.fields {
            match read_field_value(field, cx) {
                Ok(Some(v)) => {
                    map.insert(field.key.clone(), v);
                }
                Ok(None) => {}
                Err(msg) => {
                    return Err(vec![FieldError {
                        key: field.key.clone(),
                        message: msg,
                    }]);
                }
            }
        }
        Ok(map)
    }

    pub fn render(
        &mut self,
        entity: &gpui::Entity<Self>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let entity = entity.clone();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .children(self.fields.iter().enumerate().map(|(ix, field)| {
                let key = field.key.clone();
                let label = format!(
                    "{}{}",
                    field.key,
                    if field.required { " *" } else { "" }
                );
                let error = field.inline_error.clone();
                let kind_hint = kind_label(&field.kind);

                let widget: gpui::AnyElement = match &field.widget {
                    FieldWidget::String { input } => Input::new(input).into_any_element(),
                    FieldWidget::Number { input, .. } => {
                        NumberInput::new(input).into_any_element()
                    }
                    FieldWidget::Bool { value } => {
                        let checked = *value;
                        let ent = entity.clone();
                        Switch::new(("switch", ix))
                            .checked(checked)
                            .label(key.clone())
                            .on_click(move |new_checked, _, cx| {
                                ent.update(cx, |form, _| {
                                    if let Some(f) = form.fields.get_mut(ix) {
                                        if let FieldWidget::Bool { value } = &mut f.widget {
                                            *value = *new_checked;
                                        }
                                    }
                                });
                            })
                            .into_any_element()
                    }
                    FieldWidget::Enum { select } => Select::new(select).into_any_element(),
                };

                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .child(format!("{label} [{kind_hint}]")),
                    )
                    .child(widget)
                    .when_some(error, |this, err| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(gpui::rgb(0xef4444))
                                .child(err),
                        )
                    })
            }))
    }
}

fn kind_label(kind: &FieldKind) -> &'static str {
    match kind {
        FieldKind::String => "Input",
        FieldKind::Number => "NumberInput",
        FieldKind::Integer => "NumberInput/i64",
        FieldKind::Bool => "Switch",
        FieldKind::Enum(_) => "Select",
        FieldKind::Unsupported => "?",
    }
}

fn apply_default_string(
    input: &Entity<InputState>,
    default: &Option<Value>,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    if let Some(val) = default {
        let text = match val {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => val.to_string(),
        };
        input.update(cx, |s, cx| s.set_value(text, window, cx));
    }
}

fn field_value_empty(field: &FieldState, cx: &gpui::App) -> bool {
    match &field.widget {
        FieldWidget::String { input } | FieldWidget::Number { input, .. } => {
            input.read(cx).value().is_empty()
        }
        FieldWidget::Bool { .. } => false,
        FieldWidget::Enum { select } => select.read(cx).selected_value().is_none(),
    }
}

fn read_field_value(field: &FieldState, cx: &gpui::App) -> Result<Option<Value>, String> {
    match &field.widget {
        FieldWidget::String { input } => {
            let v = input.read(cx).value();
            if v.is_empty() {
                return Ok(None);
            }
            Ok(Some(Value::String(v.to_string())))
        }
        FieldWidget::Number { is_integer, input } => {
            let v = input.read(cx).value();
            if v.is_empty() {
                return Ok(None);
            }
            if *is_integer {
                v.parse::<i64>()
                    .map(|n| Some(Value::Number(n.into())))
                    .map_err(|_| "Invalid integer".into())
            } else {
                let n = v
                    .parse::<f64>()
                    .map_err(|_| "Invalid number".to_string())?;
                Ok(Some(
                    serde_json::Number::from_f64(n)
                        .map(Value::Number)
                        .ok_or_else(|| "Invalid number".to_string())?,
                ))
            }
        }
        FieldWidget::Bool { value } => Ok(Some(Value::Bool(*value))),
        FieldWidget::Enum { select } => select
            .read(cx)
            .selected_value()
            .map(|v| Some(Value::String(v.to_string())))
            .ok_or_else(|| "No selection".into()),
    }
}
