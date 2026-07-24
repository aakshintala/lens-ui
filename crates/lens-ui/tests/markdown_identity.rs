use gpui::{Context, EntityId, Render, TestAppContext, Window};
use lens_core::domain::ids::{AccId, ItemId};
use lens_ui::focused::{ContentKey, RowContent, RowId, RowKind, RowPresentation, RowStore};
use lens_ui::md::{MarkdownView, init as md_init, markdown_state_entity_id};

struct IdentityHarness {
    store: RowStore,
    before: Option<EntityId>,
    after: Option<EntityId>,
    key: ContentKey,
    acc: AccId,
    item_id: ItemId,
    done: bool,
}

impl Render for IdentityHarness {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        if !self.done {
            self.store.stage_stream_finalize(
                &self.acc,
                RowPresentation {
                    kind: RowKind::StreamingMessage,
                    content: RowContent::AssistantMarkdown {
                        source: "hi".into(),
                        content_key: self.key.clone(),
                    },
                    collapsed: false,
                    height_hint: None,
                },
                None,
                None,
                cx,
            );
            let _ = MarkdownView::new(self.key.as_element_id(), "hi", window, cx)
                .scrollable(false)
                .selectable(true);
            self.before = markdown_state_entity_id(self.key.as_element_id().as_str(), window, cx);

            self.store.commit_stream_finalize(
                &self.acc,
                &self.item_id,
                RowPresentation {
                    kind: RowKind::Message,
                    content: RowContent::AssistantMarkdown {
                        source: "hi there".into(),
                        content_key: ContentKey::from_acc(&AccId::new(self.item_id.as_str())),
                    },
                    collapsed: false,
                    height_hint: None,
                },
                true,
                None,
                cx,
            );
            let final_content = self
                .store
                .entity(&RowId::Sibling(self.item_id.clone()))
                .unwrap()
                .read(cx)
                .presentation
                .content
                .clone();
            let final_key = match &final_content {
                RowContent::AssistantMarkdown { content_key, .. } => content_key.clone(),
                other => panic!("unexpected finalized content {other:?}"),
            };
            let _ = MarkdownView::new(final_key.as_element_id(), "hi there", window, cx)
                .scrollable(false)
                .selectable(true);
            self.after = markdown_state_entity_id(final_key.as_element_id().as_str(), window, cx);
            self.done = true;
        }
        gpui::Empty
    }
}

#[gpui::test]
async fn markdown_entity_id_stable_across_finalize(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        md_init(cx);
    });
    let acc = AccId::new("acc_id_test");
    let key = ContentKey::from_acc(&acc);
    let item_id = ItemId::new("item_final_1");

    let (harness, vcx) = cx.add_window_view(|_, _| IdentityHarness {
        store: RowStore::new(),
        before: None,
        after: None,
        key: key.clone(),
        acc,
        item_id,
        done: false,
    });
    vcx.run_until_parked();

    let (before, after) = vcx.update(|_, cx| {
        let h = harness.read(cx);
        (h.before, h.after)
    });
    let before = before.expect("state before finalize");
    let after = after.expect("state after finalize");
    let _: EntityId = before;
    assert_eq!(
        before, after,
        "D11: finalize must preserve the stream key (no remount)"
    );
}
