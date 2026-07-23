//! The session-load seam (design §10 step 1).
//!
//! `FleetStore` must be able to make a brand-new session reachable when the
//! server supersedes A -> B, but it deliberately retains no `Connection`,
//! `Client`, or data dir. This trait is that seam: `lens-app` supplies the real
//! implementation (background `GET /v1/sessions/{id}` -> seed control store ->
//! `spawn_live_session`); tests supply a fake.

use crate::fleet::store::FleetStore;
use gpui::{App, Task, WeakEntity};
use lens_core::domain::ids::SessionId;

/// Makes a not-yet-tracked session reachable in a [`FleetStore`].
pub trait SessionLoader {
    /// Load `session_id` into `store`.
    ///
    /// Implementations **must not block the foreground**: the real path does a
    /// blocking HTTP GET and must run it on `cx.background_executor()` before
    /// returning to the foreground to spawn. The returned task resolves once
    /// the session is reachable (its card + poller exist) or with an error.
    ///
    /// Implementations **must not synchronously `update` the store** from
    /// inside `load`. gpui entity updates are not re-entrant, and the caller
    /// may hold an active `FleetStore` update. Do store mutation inside a
    /// spawned task (see `FakeSessionLoader` for the minimal shape).
    fn load(
        &self,
        session_id: SessionId,
        store: WeakEntity<FleetStore>,
        cx: &mut App,
    ) -> Task<Result<(), String>>;
}

#[cfg(test)]
pub(crate) struct FakeSessionLoader {
    loaded: std::cell::RefCell<Vec<SessionId>>,
    fail: bool,
}

#[cfg(test)]
impl FakeSessionLoader {
    pub(crate) fn new() -> Self {
        Self {
            loaded: std::cell::RefCell::new(Vec::new()),
            fail: false,
        }
    }

    /// A loader that always fails — proves the store does not re-parent
    /// terminals into a session it could not load.
    #[allow(dead_code)] // Task 5 supersede tests use failing loader.
    pub(crate) fn failing() -> Self {
        Self {
            loaded: std::cell::RefCell::new(Vec::new()),
            fail: true,
        }
    }

    pub(crate) fn loaded(&self) -> Vec<SessionId> {
        self.loaded.borrow().clone()
    }
}

#[cfg(test)]
impl SessionLoader for FakeSessionLoader {
    fn load(
        &self,
        session_id: SessionId,
        store: WeakEntity<FleetStore>,
        cx: &mut App,
    ) -> Task<Result<(), String>> {
        self.loaded.borrow_mut().push(session_id.clone());
        if self.fail {
            return Task::ready(Err("fake loader: forced failure".into()));
        }
        // Make B reachable the same way the fake fleet does elsewhere.
        //
        // This MUST be done in a spawned task, never synchronously here: gpui
        // entity updates are not re-entrant and `load` may be invoked while a
        // `FleetStore` update is active. Mirrors the real loader's async shape.
        cx.spawn(async move |cx| {
            store
                .update(cx, |store, cx| {
                    store.spawn_fake_session(session_id, cx);
                })
                .map_err(|e| format!("store gone: {e:?}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::store::FleetStore;
    use std::rc::Rc;
    use std::sync::Arc;

    #[gpui::test]
    async fn fake_loader_records_and_spawns(cx: &mut gpui::TestAppContext) {
        let clock = Arc::new(crate::clock::ManualUiClock::new(1_000));
        let store = cx.update(|cx| FleetStore::new(clock, cx));
        let loader = Rc::new(FakeSessionLoader::new());
        cx.update(|cx| {
            store.update(cx, |store, _cx| {
                store.set_session_loader(loader.clone());
            });
        });

        let target = SessionId::new("conv_b");
        let task = cx.update(|cx| loader.load(target.clone(), store.downgrade(), cx));
        task.await.expect("fake loader succeeds");
        cx.run_until_parked();

        assert_eq!(
            loader.loaded(),
            vec![target.clone()],
            "loader recorded the request"
        );
        cx.update(|cx| {
            assert!(
                store.read(cx).cards.contains_key(&target),
                "fake loader made session B reachable (card present)"
            );
        });
    }
}
