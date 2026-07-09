//! Source-grounded fixture corpus (⚠ derived-not-byte-verified unless noted).

use serde_json::{Map, Value, json};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixtureId {
    GenericFull,
    AskUserQuestion,
    ExitPlanMode,
    Binary,
    Url,
    Malformed,
}

impl FixtureId {
    pub const ALL: [FixtureId; 6] = [
        FixtureId::GenericFull,
        FixtureId::AskUserQuestion,
        FixtureId::ExitPlanMode,
        FixtureId::Binary,
        FixtureId::Url,
        FixtureId::Malformed,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FixtureId::GenericFull => "generic_full",
            FixtureId::AskUserQuestion => "ask_user_question",
            FixtureId::ExitPlanMode => "exit_plan_mode",
            FixtureId::Binary => "binary",
            FixtureId::Url => "url",
            FixtureId::Malformed => "malformed",
        }
    }

    pub fn next(self) -> FixtureId {
        let ix = Self::ALL.iter().position(|&f| f == self).unwrap_or(0);
        Self::ALL[(ix + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> FixtureId {
        let ix = Self::ALL.iter().position(|&f| f == self).unwrap_or(0);
        Self::ALL[(ix + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// Params shaped like omnigent `ElicitationRequestParams` (⚠ derived).
#[derive(Clone, Debug)]
pub struct Fixture {
    pub id: FixtureId,
    pub mode: Option<String>,
    pub url: Option<String>,
    pub requested_schema: Value,
    pub structured: Value,
}

impl Fixture {
    pub fn load(id: FixtureId) -> Self {
        match id {
            FixtureId::GenericFull => generic_full(),
            FixtureId::AskUserQuestion => ask_user_question(),
            FixtureId::ExitPlanMode => exit_plan_mode(),
            FixtureId::Binary => binary(),
            FixtureId::Url => url_mode(),
            FixtureId::Malformed => malformed(),
        }
    }
}

/// Flat primitive schema — subset accepted by omnigent `build_accept_content_from_schema`.
/// ⚠ derived-not-byte-verified
fn generic_full() -> Fixture {
    Fixture {
        id: FixtureId::GenericFull,
        mode: None,
        url: None,
        requested_schema: json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "count": { "type": "integer", "default": 3 },
                "ratio": { "type": "number" },
                "enabled": { "type": "boolean", "default": true },
                "color": { "type": "string", "enum": ["red", "green", "blue"] },
                "priority": {
                    "oneOf": [{ "const": "low" }, { "const": "high" }]
                }
            },
            "required": ["name", "ratio"]
        }),
        structured: Value::Null,
    }
}

/// Expected content after probe fills generic_full fields.
pub fn generic_full_expected() -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("name".into(), json!("alice"));
    m.insert("count".into(), json!(3));
    m.insert("ratio".into(), json!(1.5));
    m.insert("enabled".into(), json!(true));
    m.insert("color".into(), json!("green"));
    m.insert("priority".into(), json!("low"));
    m
}

/// Claude AskUserQuestion shape from `askUserQuestion.ts` types.
/// ⚠ derived-not-byte-verified
fn ask_user_question() -> Fixture {
    Fixture {
        id: FixtureId::AskUserQuestion,
        mode: None,
        url: None,
        requested_schema: json!({}),
        structured: json!({
            "questions": [
                {
                    "id": "framework_choice",
                    "question": "Which framework should we use?",
                    "header": "Framework",
                    "multiSelect": false,
                    "options": [
                        {
                            "label": "gpui",
                            "description": "Native Rust UI toolkit"
                        },
                        {
                            "label": "web",
                            "description": "Browser-based renderer"
                        }
                    ]
                },
                {
                    "question": "Pick deployment targets",
                    "header": "Deploy",
                    "multiSelect": true,
                    "options": [
                        {
                            "label": "macOS",
                            "description": "Desktop client"
                        },
                        {
                            "label": "Linux",
                            "description": "Server-side only"
                        }
                    ]
                }
            ]
        }),
    }
}

pub fn ask_user_question_expected() -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("framework_choice".into(), json!("gpui"));
    m.insert(
        "Pick deployment targets".into(),
        json!(["macOS", "custom-target"]),
    );
    m
}

/// ExitPlanMode structured payload.
/// ⚠ derived-not-byte-verified
fn exit_plan_mode() -> Fixture {
    Fixture {
        id: FixtureId::ExitPlanMode,
        mode: None,
        url: None,
        requested_schema: json!({}),
        structured: json!({
            "plan": "# Plan\n- step one\n- step two"
        }),
    }
}

/// Binary approve/reject (policy ASK shape).
/// ⚠ derived-not-byte-verified
fn binary() -> Fixture {
    Fixture {
        id: FixtureId::Binary,
        mode: None,
        url: None,
        requested_schema: json!({
            "type": "object",
            "properties": {
                "approve": { "type": "boolean" }
            }
        }),
        structured: Value::Null,
    }
}

/// URL mode — shape from live capture notes in spec §0.
/// ⚠ derived-not-byte-verified
fn url_mode() -> Fixture {
    Fixture {
        id: FixtureId::Url,
        mode: Some("url".into()),
        url: Some("/approve/conv_x/elicit_y".into()),
        requested_schema: json!({}),
        structured: Value::Null,
    }
}

/// Nested object property → whole-form fallback (probe 6).
/// ⚠ derived-not-byte-verified
fn malformed() -> Fixture {
    Fixture {
        id: FixtureId::Malformed,
        mode: None,
        url: None,
        requested_schema: json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "nested": {
                    "type": "object",
                    "properties": { "x": { "type": "string" } }
                }
            },
            "required": ["name"]
        }),
        structured: Value::Null,
    }
}
