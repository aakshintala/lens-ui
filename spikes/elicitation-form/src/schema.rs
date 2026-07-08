//! Runtime JSON-Schema → flat primitive field model (MCP subset).

use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldKind {
    String,
    Number,
    Integer,
    Bool,
    Enum(Vec<String>),
    Unsupported,
}

#[derive(Clone, Debug)]
pub struct ParsedField {
    pub key: String,
    pub kind: FieldKind,
    pub required: bool,
    pub default_value: Option<Value>,
}

#[derive(Clone, Debug)]
pub struct ParsedSchema {
    pub fields: Vec<ParsedField>,
    /// Any unsupported property forces the whole form onto the raw key/value fallback.
    pub use_fallback: bool,
}

pub fn parse_requested_schema(schema: &Value) -> ParsedSchema {
    let obj = schema.as_object();
    let properties = obj.and_then(|o| o.get("properties")).and_then(|p| p.as_object());
    let required: Vec<String> = obj
        .and_then(|o| o.get("required"))
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let Some(properties) = properties else {
        return ParsedSchema {
            fields: Vec::new(),
            use_fallback: false,
        };
    };

    let mut fields = Vec::new();
    let mut use_fallback = false;

    for (key, prop) in properties {
        let kind = classify_property(prop);
        if kind == FieldKind::Unsupported {
            use_fallback = true;
        }
        let default_value = prop.get("default").cloned();
        fields.push(ParsedField {
            key: key.clone(),
            kind,
            required: required.contains(key),
            default_value,
        });
    }

    fields.sort_by(|a, b| a.key.cmp(&b.key));

    ParsedSchema {
        fields,
        use_fallback,
    }
}

fn classify_property(prop: &Value) -> FieldKind {
    if let Some(one_of) = prop.get("oneOf").and_then(|v| v.as_array()) {
        let variants = extract_one_of_consts(one_of);
        if !variants.is_empty() {
            return FieldKind::Enum(variants);
        }
    }

    if let Some(enum_vals) = prop.get("enum").and_then(|v| v.as_array()) {
        let variants: Vec<String> = enum_vals
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        if !variants.is_empty() {
            return FieldKind::Enum(variants);
        }
    }

    match prop.get("type").and_then(|t| t.as_str()) {
        Some("string") => FieldKind::String,
        Some("number") => FieldKind::Number,
        Some("integer") => FieldKind::Integer,
        Some("boolean") => FieldKind::Bool,
        Some("object") | Some("array") => FieldKind::Unsupported,
        _ => FieldKind::Unsupported,
    }
}

fn extract_one_of_consts(one_of: &[Value]) -> Vec<String> {
    one_of.iter()
        .filter_map(|entry| {
            entry
                .get("const")
                .and_then(|c| c.as_str().map(String::from))
        })
        .collect()
}
