//! gpui window wiring, fixture picker, keybind-triggered probes.

use gpui::{
    actions, div, prelude::*, App, Context, FocusHandle, Focusable, KeyBinding, Styled, Window,
};
use gpui_component::ActiveTheme as _;

use crate::ask_user_question::AskUserQuestionForm;
use crate::elicitation_card::ElicitationCard;
use crate::fixtures::{self, Fixture, FixtureId};
use crate::probe::{ProbeHarness, ProbeVerdict};
use crate::schema::{parse_requested_schema, FieldKind};
use crate::schema_form::SchemaForm;

actions!(
    harness,
    [
        ProbeRuntimeForm,
        ProbeTypeCoverage,
        ProbeConstraints,
        ProbeRoundTrip,
        ProbeAskUserQuestion,
        ProbeComposition,
        FixtureNext,
        FixturePrev,
    ]
);

pub struct HarnessView {
    focus_handle: FocusHandle,
    fixture_id: FixtureId,
    card: ElicitationCard,
    probes: ProbeHarness,
    probe_form: gpui::Entity<SchemaForm>,
    headless: bool,
    ran_headless: bool,
}

impl HarnessView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window);

        let fixture_id = FixtureId::GenericFull;
        let fixture = Fixture::load(fixture_id);
        let card = ElicitationCard::from_fixture(fixture, window, cx);

        let parsed = parse_requested_schema(
            &fixtures::Fixture::load(FixtureId::GenericFull).requested_schema,
        );
        let probe_form = cx.new(|cx| SchemaForm::from_schema(&parsed, window, cx));

        Self {
            focus_handle,
            fixture_id,
            card,
            probes: ProbeHarness::new(),
            probe_form,
            headless: std::env::var("ELICIT_HEADLESS").is_ok(),
            ran_headless: false,
        }
    }

    fn reload_fixture(&mut self, id: FixtureId, window: &mut Window, cx: &mut Context<Self>) {
        self.fixture_id = id;
        let fixture = Fixture::load(id);
        self.card = ElicitationCard::from_fixture(fixture, window, cx);
        cx.notify();
    }

    fn on_fixture_next(&mut self, _: &FixtureNext, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_fixture(self.fixture_id.next(), window, cx);
    }

    fn on_fixture_prev(&mut self, _: &FixturePrev, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_fixture(self.fixture_id.prev(), window, cx);
    }

    fn run_runtime_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let expected = fixtures::generic_full_expected();
        self.probe_form.update(cx, |form, cx| {
            form.clear_errors();
            form.set_string_value("name", "alice", window, cx);
            form.set_string_value("ratio", "1.5", window, cx);
            form.set_string_value("color", "green", window, cx);
            form.set_string_value("priority", "low", window, cx);
            let count = form.fields.len();
            let ok = form.submit(cx).is_ok();
            self.probes.assert_runtime_form(count, ok);
            if let Ok(got) = form.submit(cx) {
                self.probes.assert_round_trip(&got, &expected);
            }
        });
    }

    fn on_probe_runtime(&mut self, _: &ProbeRuntimeForm, window: &mut Window, cx: &mut Context<Self>) {
        self.run_runtime_form(window, cx);
        cx.notify();
    }

    fn run_type_coverage(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.probe_form.update(cx, |form, _| {
            let kinds: Vec<_> = form
                .fields
                .iter()
                .map(|f| (f.key.clone(), f.kind.clone()))
                .collect();
            self.probes.assert_type_coverage(&kinds);
        });
    }

    fn on_probe_type_coverage(&mut self, _: &ProbeTypeCoverage, window: &mut Window, cx: &mut Context<Self>) {
        self.run_type_coverage(window, cx);
        cx.notify();
    }

    fn run_constraints(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.probe_form.update(cx, |form, cx| {
            form.clear_errors();
            form.set_string_value("name", "", window, cx);
            form.set_string_value("ratio", "", window, cx);
            let blocked = !form.can_submit(cx);

            form.set_string_value("name", "alice", window, cx);
            form.set_string_value("ratio", "not-a-number", window, cx);
            let _ = form.validate(cx);
            let parse_err = form.fields.iter().any(|f| f.inline_error.is_some());

            let default_count = form
                .fields
                .iter()
                .filter(|f| matches!(f.kind, FieldKind::Integer | FieldKind::Bool))
                .count() as u32;

            self.probes
                .assert_constraints(blocked, default_count, parse_err);
        });
    }

    fn on_probe_constraints(&mut self, _: &ProbeConstraints, window: &mut Window, cx: &mut Context<Self>) {
        self.run_constraints(window, cx);
        cx.notify();
    }

    fn run_round_trip(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let expected = fixtures::generic_full_expected();
        self.probe_form.update(cx, |form, cx| {
            form.clear_errors();
            form.set_string_value("name", "alice", window, cx);
            form.set_string_value("ratio", "1.5", window, cx);
            form.set_string_value("color", "green", window, cx);
            form.set_string_value("priority", "low", window, cx);
            match form.submit(cx) {
                Ok(got) => self.probes.assert_round_trip(&got, &expected),
                Err(_) => self.probes.round_trip = ProbeVerdict::Fail("submit failed".into()),
            }
        });
    }

    fn on_probe_round_trip(&mut self, _: &ProbeRoundTrip, window: &mut Window, cx: &mut Context<Self>) {
        self.run_round_trip(window, cx);
        cx.notify();
    }

    fn run_ask_user_question(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fixture = Fixture::load(FixtureId::AskUserQuestion);
        let expected = fixtures::ask_user_question_expected();
        if let Some(entity) = AskUserQuestionForm::new_entity(&fixture.structured, window, cx) {
            entity.update(cx, |form, cx| {
                form.fill_probe_answers(&expected, window, cx);
                match form.submit(cx) {
                    Ok(got) => self.probes.assert_ask_user_question(&got, &expected),
                    Err(e) => self.probes.ask_user_question = ProbeVerdict::Fail(e),
                }
            });
        } else {
            self.probes.ask_user_question =
                ProbeVerdict::Fail("failed to parse ask_user_question fixture".into());
        }
    }

    fn on_probe_ask_user_question(
        &mut self,
        _: &ProbeAskUserQuestion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_ask_user_question(window, cx);
        cx.notify();
    }

    fn run_composition(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.probes.assert_composition();
    }

    fn on_probe_composition(&mut self, _: &ProbeComposition, window: &mut Window, cx: &mut Context<Self>) {
        self.run_composition(window, cx);
        cx.notify();
    }

    fn print_headless_verdict(&self) {
        let p = &self.probes;
        println!(
            "PROBE runtime_form={} detail={}",
            p.runtime_form.label(),
            p.runtime_form_detail
        );
        println!(
            "PROBE type_coverage={} detail={}",
            p.type_coverage.label(),
            p.type_coverage_detail
        );
        println!(
            "PROBE constraints={} detail={}",
            p.constraints.label(),
            p.constraints_detail
        );
        println!(
            "PROBE round_trip={} detail={}",
            p.round_trip.label(),
            p.round_trip_detail
        );
        println!(
            "PROBE ask_user_question={} detail={}",
            p.ask_user_question.label(),
            p.ask_user_question_detail
        );
        println!(
            "PROBE composition={} detail={}",
            p.composition.label(),
            p.composition_detail
        );

        let passed = [
            &p.runtime_form,
            &p.type_coverage,
            &p.constraints,
            &p.round_trip,
            &p.ask_user_question,
            &p.composition,
        ]
        .iter()
        .filter(|v| matches!(v, ProbeVerdict::Pass))
        .count();
        println!("PROBE_SUMMARY passed={passed}/6");
    }

    fn maybe_run_headless(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.headless || self.ran_headless {
            return;
        }
        self.ran_headless = true;

        self.run_runtime_form(window, cx);
        self.run_type_coverage(window, cx);
        self.run_constraints(window, cx);
        self.run_round_trip(window, cx);
        self.run_ask_user_question(window, cx);
        self.run_composition(window, cx);

        self.print_headless_verdict();

        cx.on_next_frame(window, |_, _, cx| {
            cx.quit();
        });
    }

    fn render_readout(&self) -> impl IntoElement {
        let p = &self.probes;
        let line = |name: &str, v: &ProbeVerdict, detail: &str| {
            let color = match v {
                ProbeVerdict::Pass => gpui::rgb(0x22c55e),
                ProbeVerdict::Fail(_) => gpui::rgb(0xef4444),
                ProbeVerdict::Pending => gpui::rgb(0xeab308),
            };
            div()
                .text_sm()
                .text_color(color)
                .child(format!("{name}: {} — {detail}", v.label()))
        };

        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .bg(gpui::rgb(0x111111))
            .text_color(gpui::rgb(0xeeeeee))
            .child(
                div().text_sm().child(format!(
                    "fixture={} card={:?} | [←/→] fixture 1–6 probes",
                    self.fixture_id.label(),
                    self.card.kind
                )),
            )
            .child(line("1 runtime-form", &p.runtime_form, &p.runtime_form_detail))
            .child(line("2 type-coverage", &p.type_coverage, &p.type_coverage_detail))
            .child(line("3 constraints", &p.constraints, &p.constraints_detail))
            .child(line("4 round-trip", &p.round_trip, &p.round_trip_detail))
            .child(line("5 ask-user", &p.ask_user_question, &p.ask_user_question_detail))
            .child(line("6 composition", &p.composition, &p.composition_detail))
    }
}

impl Render for HarnessView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.maybe_run_headless(window, cx);

        let card_el = self.card.render(window, cx);

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_readout())
            .child(
                div()
                    .flex_grow()
                    .overflow_hidden()
                    .p_4()
                    .max_w(gpui::px(420.))
                    .child(card_el),
            )
            .key_context("Harness")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_probe_runtime))
            .on_action(cx.listener(Self::on_probe_type_coverage))
            .on_action(cx.listener(Self::on_probe_constraints))
            .on_action(cx.listener(Self::on_probe_round_trip))
            .on_action(cx.listener(Self::on_probe_ask_user_question))
            .on_action(cx.listener(Self::on_probe_composition))
            .on_action(cx.listener(Self::on_fixture_next))
            .on_action(cx.listener(Self::on_fixture_prev))
    }
}

impl Focusable for HarnessView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

pub fn register_keybindings(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("1", ProbeRuntimeForm, Some("Harness")),
        KeyBinding::new("2", ProbeTypeCoverage, Some("Harness")),
        KeyBinding::new("3", ProbeConstraints, Some("Harness")),
        KeyBinding::new("4", ProbeRoundTrip, Some("Harness")),
        KeyBinding::new("5", ProbeAskUserQuestion, Some("Harness")),
        KeyBinding::new("6", ProbeComposition, Some("Harness")),
        KeyBinding::new("right", FixtureNext, Some("Harness")),
        KeyBinding::new("left", FixturePrev, Some("Harness")),
        KeyBinding::new("[", FixturePrev, Some("Harness")),
        KeyBinding::new("]", FixtureNext, Some("Harness")),
    ]);
}
