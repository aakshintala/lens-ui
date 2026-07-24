//! gpui `list()` render surface for the focused transcript (T-2 §7).

use crate::focused::{FocusedTranscript, RowKind, RowPresentation};
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, IntoElement, ListOffset, ListScrollEvent,
    ParentElement, Render, Styled, Window, div, list, prelude::*, px,
};

/// Auto-follow ↔ paused (§16 contract 1).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FollowMode {
    #[default]
    Following,
    Paused,
}

pub struct FocusedTranscriptView {
    replica: Entity<FocusedTranscript>,
    follow_mode: FollowMode,
    /// Row count when follow last transitioned to `Paused` — drives pill `N`.
    rows_at_pause: usize,
    last_row_count: usize,
    focus_handle: FocusHandle,
}

impl FocusedTranscriptView {
    pub fn new(replica: Entity<FocusedTranscript>, cx: &mut Context<Self>) -> Self {
        let row_count = replica.read(cx).rows().len();
        replica.update(cx, |r, _| {
            r.list_state_mut().reset(row_count);
        });
        let focus_handle = cx.focus_handle();
        // The observe callback is already invoked with the view leased (&mut Context<Self>);
        // notify directly. A nested weak.update() here re-enters the same lease → panic.
        cx.observe(&replica, move |_, _, cx| {
            cx.notify();
        })
        .detach();
        Self {
            replica,
            follow_mode: FollowMode::Following,
            rows_at_pause: row_count,
            last_row_count: row_count,
            focus_handle,
        }
    }

    fn list_state(&self, cx: &App) -> gpui::ListState {
        self.replica.read(cx).list_state().clone()
    }

    fn row_count(&self, cx: &App) -> usize {
        self.replica.read(cx).rows().len()
    }

    fn set_follow_mode(&mut self, mode: FollowMode, row_count: usize) {
        if mode == self.follow_mode {
            return;
        }
        self.follow_mode = mode;
        if mode == FollowMode::Paused {
            self.rows_at_pause = row_count;
        }
    }

    fn note_row_count(&mut self, row_count: usize) {
        self.last_row_count = row_count;
    }

    fn new_since_pause(&self, cx: &App) -> usize {
        if self.follow_mode == FollowMode::Paused {
            let replica = self.replica.read(cx);
            let hi = replica.resident_hi();
            let committed = replica.known_committed();
            if hi >= 0 && committed > hi {
                (committed - hi) as usize
            } else {
                0
            }
        } else {
            0
        }
    }

    /// Follow-mode decision for a list scroll event (§16 contract 1).
    ///
    /// For a `ListAlignment::Bottom` list, `is_scrolled` (== `logical_scroll_top
    /// .is_some()`) is the single source of truth: `false` ⟺ pinned to the
    /// bottom ⟺ Following; `true` ⟺ scrolled up ⟺ Paused. `event.visible_range`
    /// must NOT gate this — gpui computes it from the *pre-scroll* offset while
    /// `is_scrolled` is *post-scroll* (list.rs `scroll()`), so a single wheel
    /// delta that lands at the bottom yields a mid-list range + `is_scrolled =
    /// false`; gating resume on the range would wrongly keep the reader paused.
    ///
    /// Extracted from the render closure so it is directly unit-testable: gpui
    /// does not permit synthesizing a real `ScrollWheelEvent` outside its own
    /// `test-support` (`Window::dispatch_event` returns a `pub(crate)` type), so
    /// the real-window probe cannot fire this path. The render closure is
    /// trivial glue over this method; branches are covered by `on_scroll_event_*`.
    fn on_scroll_event(&mut self, event: &ListScrollEvent, cx: &mut Context<Self>) {
        let following = !event.is_scrolled;
        let mode = if following {
            FollowMode::Following
        } else {
            FollowMode::Paused
        };
        self.set_follow_mode(mode, self.last_row_count);
        self.replica.update(cx, |r, cx| {
            r.set_following(following, cx);
            r.page_older_if_near_top(event.visible_range.start, cx);
        });
    }

    fn jump_to_latest(&mut self, cx: &mut Context<Self>) {
        self.replica.update(cx, |r, cx| {
            r.set_following(true, cx);
            r.request_tail_reload();
        });
        let count = self.row_count(cx);
        self.replica.read(cx).list_state().scroll_to(ListOffset {
            item_ix: count,
            offset_in_item: px(0.),
        });
        self.follow_mode = FollowMode::Following;
        self.rows_at_pause = count;
        self.last_row_count = count;
        cx.notify();
    }

    fn render_stub_row(pres: &RowPresentation, ix: usize) -> gpui::AnyElement {
        div()
            .id(ix)
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .child(
                div()
                    .text_xs()
                    .text_color(gpui::rgb(0x888888))
                    .child(kind_tag(pres.kind)),
            )
            .child(pres.text.clone())
            .when_some(pres.height_hint, |el, h| el.h(px(h)))
            .into_any_element()
    }

    #[doc(hidden)]
    pub fn follow_mode(&self) -> FollowMode {
        self.follow_mode
    }

    #[doc(hidden)]
    pub fn new_since_pause_for_test(&self, cx: &App) -> usize {
        self.new_since_pause(cx)
    }

    #[doc(hidden)]
    pub fn pill_visible_for_test(&self, cx: &App) -> bool {
        self.follow_mode == FollowMode::Paused && self.new_since_pause(cx) > 0
    }

    #[doc(hidden)]
    pub fn jump_to_latest_for_test(&mut self, cx: &mut Context<Self>) {
        self.jump_to_latest(cx);
    }
}

fn kind_tag(kind: RowKind) -> &'static str {
    match kind {
        RowKind::SectionChip => "SectionChip",
        RowKind::SectionRail => "SectionRail",
        RowKind::WorkChild => "WorkChild",
        RowKind::Message => "Message",
        RowKind::UserMessage => "UserMessage",
        RowKind::ResourceEvent => "ResourceEvent",
        RowKind::StreamingReasoning => "StreamingReasoning",
        RowKind::StreamingMessage => "StreamingMessage",
        RowKind::ReconnectBreak => "ReconnectBreak",
        RowKind::LoadOlder => "LoadOlder",
    }
}

impl Render for FocusedTranscriptView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row_count = self.row_count(cx);
        self.note_row_count(row_count);

        let list_state = self.list_state(cx);
        let weak = cx.weak_entity();
        // Scroll events fire from input, outside the render/update pass, so a direct weak.update
        // is not re-entrant (the re-entrancy panic was the replica observer, fixed in `new`).
        list_state.set_scroll_handler(move |event: &ListScrollEvent, _, app| {
            weak.update(app, |view, cx| view.on_scroll_event(event, cx))
                .ok();
        });

        let replica = self.replica.clone();
        let list_el = list(list_state, move |ix, _window, app| {
            let replica = replica.clone();
            let Some(id) = replica.read(app).rows().id_at(ix) else {
                return div().into_any_element();
            };
            let Some(entity) = replica.read(app).rows().entity(id) else {
                return div().into_any_element();
            };
            let pres = entity.read(app).presentation.clone();
            FocusedTranscriptView::render_stub_row(&pres, ix)
        })
        .size_full();

        let pill = if self.follow_mode == FollowMode::Paused {
            let n = self.new_since_pause(cx);
            if n > 0 {
                Some(
                    div()
                        .id("jump-to-latest-pill")
                        .absolute()
                        .bottom(px(12.))
                        .left(px(12.))
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(gpui::rgb(0x2563eb))
                        .text_color(gpui::rgb(0xffffff))
                        .cursor_pointer()
                        .child(format!("↓ {n} new · jump to latest"))
                        .on_click(cx.listener(|view: &mut Self, _: &ClickEvent, _window, cx| {
                            view.jump_to_latest(cx);
                        })),
                )
            } else {
                None
            }
        } else {
            None
        };

        let syncing = if self.replica.read(cx).syncing() {
            Some(
                div()
                    .id("syncing-indicator")
                    .absolute()
                    .top(px(12.))
                    .right(px(12.))
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(gpui::rgb(0x374151))
                    .text_color(gpui::rgb(0xe5e7eb))
                    .text_sm()
                    .child("syncing…"),
            )
        } else {
            None
        };

        div()
            .id("focused-transcript-view")
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .child(list_el)
            .children(pill)
            .children(syncing)
    }
}

/// Build a `TabHandle` whose view is the focused transcript `list()` surface.
pub fn mount_focused_transcript_view(
    replica: Entity<FocusedTranscript>,
    cx: &mut App,
) -> (Entity<FocusedTranscriptView>, FocusHandle) {
    let view = cx.new(|cx| FocusedTranscriptView::new(replica.clone(), cx));
    let focus_handle = view.read(cx).focus_handle.clone();
    (view, focus_handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::store::ReconcileEpoch;
    use crate::focused::{
        FocusedTranscript, ReaderWorkerHandle, RowId, RowKind, RowPresentation, RowStore,
    };
    use gpui::{ListAlignment, ListState};
    use lens_core::domain::ids::{AccId, CallId, ItemId, ResponseId};
    use lens_core::domain::item::{
        BlockContext, ContentBlock, Item, ItemKind, MessageAcc, ReasoningAcc, StreamScratch,
    };
    use lens_core::domain::scalars::Role;
    use lens_core::reduce::{group_work_section, project};
    use serde_json::Value;

    fn ctx_with(resp: Option<&str>) -> BlockContext {
        BlockContext {
            agent: None,
            depth: 0,
            response_id: resp.map(ResponseId::new),
        }
    }

    fn assistant(id: &str, text: &str, resp: &str) -> Item {
        Item {
            id: ItemId::new(id),
            seq: None,
            ctx: ctx_with(Some(resp)),
            created_at: 0,
            kind: ItemKind::Message {
                role: Role::Assistant,
                content: vec![ContentBlock {
                    kind: "text".into(),
                    text: Some(text.into()),
                    data: Value::Null,
                }],
            },
        }
    }

    fn user(id: &str, text: &str) -> Item {
        Item {
            id: ItemId::new(id),
            seq: None,
            ctx: ctx_with(None),
            created_at: 0,
            kind: ItemKind::Message {
                role: Role::User,
                content: vec![ContentBlock {
                    kind: "text".into(),
                    text: Some(text.into()),
                    data: Value::Null,
                }],
            },
        }
    }

    fn reasoning(id: &str, resp: &str) -> Item {
        Item {
            id: ItemId::new(id),
            seq: None,
            ctx: ctx_with(Some(resp)),
            created_at: 0,
            kind: ItemKind::Reasoning {
                full_text: "think".into(),
                summary_text: String::new(),
                encrypted: false,
            },
        }
    }

    fn tool_call(id: &str, resp: &str, call_id: &str) -> Item {
        Item {
            id: ItemId::new(id),
            seq: None,
            ctx: ctx_with(Some(resp)),
            created_at: 0,
            kind: ItemKind::FunctionCall {
                call_id: CallId::new(call_id),
                name: "read".into(),
                arguments: Value::Null,
                status: "completed".into(),
                agent_name: None,
            },
        }
    }

    fn tool_output(id: &str, resp: &str, call_id: &str) -> Item {
        Item {
            id: ItemId::new(id),
            seq: None,
            ctx: ctx_with(Some(resp)),
            created_at: 0,
            kind: ItemKind::FunctionCallOutput {
                call_id: CallId::new(call_id),
                output: "ok".into(),
                arguments: Value::Null,
            },
        }
    }

    fn resource(id: &str) -> Item {
        use lens_client::generated::{SessionResourceObject, Type};
        Item {
            id: ItemId::new(id),
            seq: None,
            ctx: ctx_with(None),
            created_at: 0,
            kind: ItemKind::ResourceEvent {
                resource: SessionResourceObject {
                    environment: None,
                    id: "default".into(),
                    metadata: serde_json::Map::new(),
                    name: "file.rs".into(),
                    object: "session.resource".into(),
                    session_id: "conv_1".into(),
                    type_: Type::Environment,
                },
            },
        }
    }

    fn new_replica(cx: &mut gpui::TestAppContext) -> gpui::Entity<FocusedTranscript> {
        let (reader, _rx) = ReaderWorkerHandle::new_test();
        let session_id = lens_core::domain::ids::SessionId::new("sess_view_test");
        cx.update(|cx| {
            cx.new(|cx| {
                FocusedTranscript::new_test_no_baseline(
                    reader,
                    session_id,
                    ReconcileEpoch::default(),
                    1,
                    cx,
                )
            })
        })
    }

    /// Materialize one of each `ViewBlock` variant plus a `ReconnectBreak` marker row.
    fn materialize_all_row_kinds(cx: &mut gpui::TestAppContext) -> (RowStore, gpui::ListState) {
        let resp = ResponseId::new("resp_a");
        let items = [
            reasoning("r1", "resp_a"),
            tool_call("c1", "resp_a", "call_1"),
            tool_output("o1", "resp_a", "call_1"),
            user("u1", "hello"),
            resource("res1"),
            assistant("a1", "assistant msg", "resp_a"),
        ];
        let refs: Vec<&Item> = items.iter().collect();
        let scratch = StreamScratch {
            open_reasoning: Some(ReasoningAcc {
                acc_id: AccId::new("acc_r"),
                full_text: "streaming reasoning".into(),
                summary_text: String::new(),
                encrypted: false,
            }),
            open_message: Some(MessageAcc {
                acc_id: AccId::new("acc_m"),
                message_id: None,
                text: "streaming message".into(),
                block_index: 0,
            }),
            ..Default::default()
        };
        let projected = project(&refs, &scratch, Some(&resp));
        let blocks = group_work_section(projected, Some(&resp));

        let mut store = RowStore::new();
        cx.update(|cx| RowStore::materialize_full(&blocks, &mut store, cx));
        cx.update(|cx| {
            let marker_id = RowId::Marker(42);
            store.upsert(
                marker_id.clone(),
                RowPresentation {
                    kind: RowKind::ReconnectBreak,
                    text: "reconnect".into(),
                    collapsed: false,
                    height_hint: None,
                },
                cx,
            );
            store
                .structure
                .push(crate::focused::rowsource::StructureEntry::Marker(marker_id));
            store.rebuild_flat_order();
        });

        let list = ListState::new(0, ListAlignment::Bottom, px(200.));
        list.reset(store.len());
        (store, list)
    }

    #[gpui::test]
    fn every_row_kind_materializes_without_panic(cx: &mut gpui::TestAppContext) {
        let (store, list) = materialize_all_row_kinds(cx);
        let kinds: Vec<RowKind> = cx.read(|cx| {
            (0..store.len())
                .map(|ix| store.kind_at(ix, cx).expect("kind"))
                .collect()
        });

        let expected = [
            RowKind::SectionRail,
            RowKind::WorkChild,
            RowKind::WorkChild,
            RowKind::UserMessage,
            RowKind::ResourceEvent,
            RowKind::Message,
            RowKind::SectionRail,
            RowKind::StreamingReasoning,
            RowKind::StreamingMessage,
            RowKind::ReconnectBreak,
        ];
        assert_eq!(kinds, expected);
        assert_eq!(list.item_count(), store.len());
    }

    #[gpui::test]
    fn focused_view_mount_resets_list_to_bottom(cx: &mut gpui::TestAppContext) {
        let replica = new_replica(cx);
        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                let mut store = RowStore::new();
                for i in 0..5 {
                    let id = RowId::Sibling(ItemId::new(format!("m{i}")));
                    store.upsert(
                        id.clone(),
                        RowPresentation {
                            kind: RowKind::Message,
                            text: format!("row {i}"),
                            collapsed: false,
                            height_hint: None,
                        },
                        cx,
                    );
                    store
                        .structure
                        .push(crate::focused::rowsource::StructureEntry::Sibling(id));
                }
                store.rebuild_flat_order();
                *r.rows_mut() = store;
            });
        });

        let view = cx.update(|cx| cx.new(|cx| FocusedTranscriptView::new(replica.clone(), cx)));

        cx.read(|cx| {
            let count = replica.read(cx).rows().len();
            let offset = replica.read(cx).list_state().logical_scroll_top();
            assert_eq!(replica.read(cx).list_state().item_count(), count);
            assert_eq!(offset.item_ix, count);
            assert_eq!(offset.offset_in_item, px(0.));
            assert_eq!(view.read(cx).follow_mode(), FollowMode::Following);
        });
    }

    #[gpui::test]
    fn follow_mode_pill_n_counts_committed_unresident(cx: &mut gpui::TestAppContext) {
        let replica = new_replica(cx);
        cx.update(|cx| {
            replica.update(cx, |r, _| {
                r.known_committed = 10;
                r.resident_hi = 7;
            });
        });
        let view = cx.update(|cx| cx.new(|cx| FocusedTranscriptView::new(replica.clone(), cx)));

        cx.update(|cx| {
            view.update(cx, |v, _| {
                v.set_follow_mode(FollowMode::Paused, 3);
            });
        });

        cx.read(|cx| {
            let v = view.read(cx);
            assert_eq!(v.follow_mode(), FollowMode::Paused);
            assert_eq!(v.new_since_pause_for_test(cx), 3);
            assert!(v.pill_visible_for_test(cx));
        });

        cx.update(|cx| {
            view.update(cx, |v, cx| v.jump_to_latest_for_test(cx));
        });

        cx.read(|cx| {
            let v = view.read(cx);
            assert_eq!(v.follow_mode(), FollowMode::Following);
            assert!(!v.pill_visible_for_test(cx));
            assert!(
                replica.read(cx).is_following(),
                "jump to latest must restore replica following"
            );
        });
    }

    /// Build a mounted view over a replica seeded with `n` message rows.
    fn view_with_rows(
        cx: &mut gpui::TestAppContext,
        n: usize,
    ) -> (
        gpui::Entity<FocusedTranscript>,
        gpui::Entity<FocusedTranscriptView>,
    ) {
        let replica = new_replica(cx);
        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                for i in 0..n {
                    let id = RowId::Sibling(ItemId::new(format!("m{i}")));
                    r.rows_mut().upsert(
                        id.clone(),
                        RowPresentation {
                            kind: RowKind::Message,
                            text: format!("row {i}"),
                            collapsed: false,
                            height_hint: None,
                        },
                        cx,
                    );
                    r.rows_mut()
                        .structure
                        .push(crate::focused::rowsource::StructureEntry::Sibling(id));
                }
                r.rows_mut().rebuild_flat_order();
            });
        });
        let view = cx.update(|cx| cx.new(|cx| FocusedTranscriptView::new(replica.clone(), cx)));
        (replica, view)
    }

    fn scroll_event(visible_end: usize, count: usize, is_scrolled: bool) -> gpui::ListScrollEvent {
        gpui::ListScrollEvent {
            visible_range: 0..visible_end,
            count,
            is_scrolled,
        }
    }

    // The four scroll contracts' follow-mode decisions live here because gpui
    // walls off synthetic ScrollWheelEvent injection outside its own
    // test-support (see `on_scroll_event`); the real-window probe covers only
    // the paint-level anchor/overflow behavior.

    #[gpui::test]
    fn on_scroll_event_pauses_when_scrolled_up(cx: &mut gpui::TestAppContext) {
        let (replica, view) = view_with_rows(cx, 5);
        cx.update(|cx| {
            view.update(cx, |v, cx| {
                v.note_row_count(5);
                // Scrolled up: visible window ends mid-list, is_scrolled = true.
                v.on_scroll_event(&scroll_event(2, 5, true), cx);
            });
        });
        cx.read(|cx| {
            let v = view.read(cx);
            assert_eq!(v.follow_mode(), FollowMode::Paused);
            // Pause captures the row count so the pill counts appends since.
            assert_eq!(v.new_since_pause_for_test(cx), 0);
            assert!(
                !replica.read(cx).is_following(),
                "scroll pause must clear replica following"
            );
        });
    }

    #[gpui::test]
    fn on_scroll_event_resumes_when_back_at_bottom(cx: &mut gpui::TestAppContext) {
        let (replica, view) = view_with_rows(cx, 5);
        cx.update(|cx| {
            view.update(cx, |v, cx| {
                v.note_row_count(5);
                v.on_scroll_event(&scroll_event(2, 5, true), cx); // pause
                // Back at bottom: not scrolled (logical_scroll_top == None).
                v.on_scroll_event(&scroll_event(5, 5, false), cx);
            });
        });
        cx.read(|cx| {
            assert_eq!(view.read(cx).follow_mode(), FollowMode::Following);
            assert!(
                replica.read(cx).is_following(),
                "scroll resume must set replica following"
            );
        });
    }

    #[gpui::test]
    fn on_scroll_event_at_bottom_stays_following(cx: &mut gpui::TestAppContext) {
        let (replica, view) = view_with_rows(cx, 5);
        cx.update(|cx| {
            view.update(cx, |v, cx| {
                v.note_row_count(5);
                v.on_scroll_event(&scroll_event(5, 5, false), cx);
            });
        });
        cx.read(|cx| {
            assert_eq!(view.read(cx).follow_mode(), FollowMode::Following);
            assert!(replica.read(cx).is_following());
        });
    }

    #[gpui::test]
    fn on_scroll_event_bottom_landing_resumes_despite_prescroll_range(
        cx: &mut gpui::TestAppContext,
    ) {
        // gpui reports `visible_range` PRE-scroll but `is_scrolled` POST-scroll.
        // A single wheel delta that lands at the bottom therefore arrives as a
        // mid-list range + is_scrolled=false. For a Bottom-aligned list that
        // means "pinned to bottom" → MUST resume Following. Keying resume on the
        // (stale) range would wrongly keep the reader paused. (Codex finding.)
        let (replica, view) = view_with_rows(cx, 5);
        cx.update(|cx| {
            view.update(cx, |v, cx| {
                v.note_row_count(5);
                v.on_scroll_event(&scroll_event(2, 5, true), cx); // pause
                // Pre-scroll range still mid-list (2), but not scrolled anymore.
                v.on_scroll_event(&scroll_event(2, 5, false), cx);
            });
        });
        cx.read(|cx| {
            assert_eq!(view.read(cx).follow_mode(), FollowMode::Following);
            assert!(
                replica.read(cx).is_following(),
                "bottom landing must restore replica following"
            );
        });
    }

    #[gpui::test]
    fn stub_renderer_covers_every_row_kind(_cx: &mut gpui::TestAppContext) {
        for kind in [
            RowKind::SectionChip,
            RowKind::SectionRail,
            RowKind::WorkChild,
            RowKind::Message,
            RowKind::UserMessage,
            RowKind::ResourceEvent,
            RowKind::StreamingReasoning,
            RowKind::StreamingMessage,
            RowKind::ReconnectBreak,
            RowKind::LoadOlder,
        ] {
            let _ = FocusedTranscriptView::render_stub_row(
                &RowPresentation {
                    kind,
                    text: "stub".into(),
                    collapsed: false,
                    height_hint: None,
                },
                0,
            );
        }
    }

    #[gpui::test]
    fn jump_to_latest_enqueues_tail_reload_when_evicted(cx: &mut gpui::TestAppContext) {
        use crate::focused::reader::Priority;
        use lens_core::persist::ReadRange;

        let (reader, rx) = ReaderWorkerHandle::new_test();
        let session_id = lens_core::domain::ids::SessionId::new("sess_jump_tail");
        let replica = cx.update(|cx| {
            cx.new(|cx| {
                FocusedTranscript::new_test_no_baseline(
                    reader,
                    session_id,
                    ReconcileEpoch::default(),
                    1,
                    cx,
                )
            })
        });
        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                for i in 0..3 {
                    let id = RowId::Sibling(ItemId::new(format!("m{i}")));
                    r.rows_mut().upsert(
                        id.clone(),
                        RowPresentation {
                            kind: RowKind::Message,
                            text: format!("row {i}"),
                            collapsed: false,
                            height_hint: None,
                        },
                        cx,
                    );
                    r.rows_mut()
                        .structure
                        .push(crate::focused::rowsource::StructureEntry::Sibling(id));
                }
                r.rows_mut().rebuild_flat_order();
                r.known_committed = 10;
                r.resident_hi = 7;
            });
        });
        let view = cx.update(|cx| cx.new(|cx| FocusedTranscriptView::new(replica.clone(), cx)));

        cx.update(|cx| {
            view.update(cx, |v, cx| v.jump_to_latest_for_test(cx));
        });

        let tail = rx.try_recv().expect("tail reload on evicted jump");
        assert!(matches!(tail.range, ReadRange::Tail { .. }));
        assert_eq!(tail.priority, Priority::Delta);
        cx.read(|cx| {
            assert!(replica.read(cx).is_following());
        });
    }

    #[gpui::test]
    fn on_scroll_near_top_enqueues_single_backward_page(cx: &mut gpui::TestAppContext) {
        use crate::focused::reader::Priority;
        use lens_core::persist::ReadRange;

        let (reader, rx) = ReaderWorkerHandle::new_test();
        let session_id = lens_core::domain::ids::SessionId::new("sess_page_older");
        let replica = cx.update(|cx| {
            cx.new(|cx| {
                FocusedTranscript::new_test_no_baseline(
                    reader,
                    session_id,
                    ReconcileEpoch::default(),
                    1,
                    cx,
                )
            })
        });
        cx.update(|cx| {
            replica.update(cx, |r, cx| {
                for i in 5..10 {
                    let id = RowId::Sibling(ItemId::new(format!("m{i}")));
                    r.rows_mut().upsert(
                        id.clone(),
                        RowPresentation {
                            kind: RowKind::Message,
                            text: format!("row {i}"),
                            collapsed: false,
                            height_hint: None,
                        },
                        cx,
                    );
                    r.rows_mut()
                        .structure
                        .push(crate::focused::rowsource::StructureEntry::Sibling(id));
                }
                r.rows_mut().rebuild_flat_order();
                r.resident_lo = 5;
                r.resident_hi = 9;
            });
        });
        let view = cx.update(|cx| cx.new(|cx| FocusedTranscriptView::new(replica.clone(), cx)));

        cx.update(|cx| {
            view.update(cx, |v, cx| {
                v.on_scroll_event(&scroll_event(2, 6, true), cx);
            });
        });

        let page = rx.try_recv().expect("backward page near top");
        assert_eq!(
            page.range,
            ReadRange::Backward {
                before: 5,
                byte_budget: 4 * 1024 * 1024,
            }
        );
        assert_eq!(page.priority, Priority::Page);
        cx.read(|cx| {
            assert!(replica.read(cx).page_in_flight_for_test());
        });

        cx.update(|cx| {
            view.update(cx, |v, cx| {
                v.on_scroll_event(&scroll_event(1, 6, true), cx);
            });
        });
        assert!(
            rx.try_recv().is_err(),
            "second near-top scroll must not enqueue while page in flight"
        );
    }
}
