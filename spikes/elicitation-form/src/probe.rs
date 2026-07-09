//! Probe harness — baked-in assertions + on-screen PASS/FAIL readout.

use serde_json::{Map, Value};

use crate::elicitation_card::{CardKind, card_kind_for_fixture};
use crate::fixtures::{self, FixtureId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProbeVerdict {
    Pending,
    Pass,
    Fail(String),
}

impl ProbeVerdict {
    pub fn label(&self) -> &'static str {
        match self {
            ProbeVerdict::Pending => "PENDING",
            ProbeVerdict::Pass => "PASS",
            ProbeVerdict::Fail(_) => "FAIL",
        }
    }
}

#[derive(Debug)]
pub struct ProbeHarness {
    pub runtime_form: ProbeVerdict,
    pub runtime_form_detail: String,

    pub type_coverage: ProbeVerdict,
    pub type_coverage_detail: String,

    pub constraints: ProbeVerdict,
    pub constraints_detail: String,

    pub round_trip: ProbeVerdict,
    pub round_trip_detail: String,

    pub ask_user_question: ProbeVerdict,
    pub ask_user_question_detail: String,

    pub composition: ProbeVerdict,
    pub composition_detail: String,
}

impl ProbeHarness {
    pub fn new() -> Self {
        Self {
            runtime_form: ProbeVerdict::Pending,
            runtime_form_detail: "press 1 — build N fields from schema, read-back".into(),
            type_coverage: ProbeVerdict::Pending,
            type_coverage_detail: "press 2 — widget kind per FieldKind".into(),
            constraints: ProbeVerdict::Pending,
            constraints_detail: "press 3 — required/default/parse errors".into(),
            round_trip: ProbeVerdict::Pending,
            round_trip_detail: "press 4 — JSON content compare".into(),
            ask_user_question: ProbeVerdict::Pending,
            ask_user_question_detail: "press 5 — carousel flat map".into(),
            composition: ProbeVerdict::Pending,
            composition_detail: "press 6 — discriminator + fallback routing".into(),
        }
    }

    pub fn assert_runtime_form(&mut self, field_count: usize, submit_ok: bool) {
        self.runtime_form_detail =
            format!("built {field_count} runtime fields; submit_ok={submit_ok}");
        self.runtime_form = if field_count >= 6 && submit_ok {
            ProbeVerdict::Pass
        } else {
            ProbeVerdict::Fail(format!(
                "expected ≥6 fields + successful submit, got {field_count} fields submit_ok={submit_ok}"
            ))
        };
    }

    pub fn assert_type_coverage(&mut self, kinds: &[(String, crate::schema::FieldKind)]) {
        let mut issues = Vec::new();
        for (key, kind) in kinds {
            if matches!(kind, crate::schema::FieldKind::Unsupported) {
                issues.push(format!("{key}: unsupported"));
            }
        }
        let detail: Vec<String> = kinds
            .iter()
            .map(|(k, kind)| format!("{k}={kind:?}"))
            .collect();
        self.type_coverage_detail = detail.join(", ");
        self.type_coverage = if issues.is_empty() && kinds.len() >= 6 {
            ProbeVerdict::Pass
        } else {
            ProbeVerdict::Fail(issues.join("; "))
        };
    }

    pub fn assert_constraints(
        &mut self,
        required_blocks: bool,
        default_count: u32,
        parse_error_shown: bool,
    ) {
        self.constraints_detail = format!(
            "required_blocks_submit={required_blocks} defaults_prefilled={default_count} parse_error_inline={parse_error_shown}"
        );
        self.constraints = if required_blocks && default_count >= 2 && parse_error_shown {
            ProbeVerdict::Pass
        } else {
            ProbeVerdict::Fail("constraint checks incomplete".into())
        };
    }

    pub fn assert_round_trip(&mut self, got: &Map<String, Value>, expected: &Map<String, Value>) {
        let got_json = Value::Object(got.clone());
        let exp_json = Value::Object(expected.clone());
        let match_ok = got_json == exp_json;
        self.round_trip_detail = format!(
            "got={} expected={}",
            serde_json::to_string(&got_json).unwrap_or_default(),
            serde_json::to_string(&exp_json).unwrap_or_default()
        );
        self.round_trip = if match_ok {
            ProbeVerdict::Pass
        } else {
            ProbeVerdict::Fail("content mismatch".into())
        };
    }

    pub fn assert_ask_user_question(
        &mut self,
        got: &Map<String, Value>,
        expected: &Map<String, Value>,
    ) {
        // Multi-select answers are an unordered `list[str]` set (MCP content is
        // order-insignificant), so canonicalize array order before comparing —
        // otherwise the form's sorted output false-FAILs an insertion-ordered
        // oracle. (Ordering choice itself is recorded in the verdict doc.)
        let got_json = canonicalize_arrays(&Value::Object(got.clone()));
        let exp_json = canonicalize_arrays(&Value::Object(expected.clone()));
        self.ask_user_question_detail = format!(
            "got={} expected={}",
            serde_json::to_string(&got_json).unwrap_or_default(),
            serde_json::to_string(&exp_json).unwrap_or_default()
        );
        self.ask_user_question = if got_json == exp_json {
            ProbeVerdict::Pass
        } else {
            ProbeVerdict::Fail("answer map mismatch".into())
        };
    }

    pub fn assert_composition(&mut self) {
        let mut mismatches = Vec::new();
        let expectations: [(FixtureId, CardKind); 6] = [
            (FixtureId::GenericFull, CardKind::SchemaForm),
            (FixtureId::AskUserQuestion, CardKind::AskUserQuestion),
            (FixtureId::ExitPlanMode, CardKind::PlanReview),
            (FixtureId::Binary, CardKind::Binary),
            (FixtureId::Url, CardKind::Url),
            (FixtureId::Malformed, CardKind::RawFallback),
        ];
        for (id, expected) in expectations {
            let fixture = fixtures::Fixture::load(id);
            let got = card_kind_for_fixture(&fixture);
            if got != expected {
                mismatches.push(format!("{}: got {got:?} want {expected:?}", id.label()));
            }
        }
        self.composition_detail = if mismatches.is_empty() {
            "all 6 fixtures routed to expected card kind".into()
        } else {
            mismatches.join("; ")
        };
        self.composition = if mismatches.is_empty() {
            ProbeVerdict::Pass
        } else {
            ProbeVerdict::Fail(mismatches.join("; "))
        };
    }
}

impl Default for ProbeHarness {
    fn default() -> Self {
        Self::new()
    }
}

/// Recursively sort every `Value::Array` of strings so comparison is
/// order-insensitive — multi-select answers are a set, not a sequence.
fn canonicalize_arrays(v: &Value) -> Value {
    match v {
        Value::Array(items) => {
            let mut mapped: Vec<Value> = items.iter().map(canonicalize_arrays).collect();
            mapped.sort_by(|a, b| {
                serde_json::to_string(a)
                    .unwrap_or_default()
                    .cmp(&serde_json::to_string(b).unwrap_or_default())
            });
            Value::Array(mapped)
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, val)| (k.clone(), canonicalize_arrays(val)))
                .collect(),
        ),
        other => other.clone(),
    }
}
