//! The real [`SessionLoader`]: makes a brand-new session reachable mid-stream.
//!
//! `session.superseded` hands us a conversation id we have never seen. Before
//! `FleetStore::spawn_live_session` can run, that session must exist in the
//! control store — `scheduler.reconnect` does `load_session(..).ok_or(
//! SessionNotFound)`. So we GET the snapshot and seed it first. The GET is
//! blocking, so it runs on the background executor; only the spawn returns to
//! the foreground.

use gpui::{App, AsyncApp, Task, WeakEntity};
use lens_client::sessions::GetOpts; // NOT re-exported at the lens_client crate root
use lens_client::{Client, Connection};
use lens_core::domain::ids::SessionId;
use lens_ui::fleet::loader::SessionLoader;
use lens_ui::fleet::store::FleetStore;
use std::path::PathBuf;

pub(crate) struct AppSessionLoader {
    conn: Connection,
    data_dir: PathBuf,
}

impl AppSessionLoader {
    pub(crate) fn new(conn: Connection, data_dir: PathBuf) -> Self {
        Self { conn, data_dir }
    }
}

impl SessionLoader for AppSessionLoader {
    fn load(
        &self,
        session_id: SessionId,
        store: WeakEntity<FleetStore>,
        cx: &mut App,
    ) -> Task<Result<(), String>> {
        let conn = self.conn.clone();
        let data_dir = self.data_dir.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            // Blocking GET + control-store seed, off the foreground.
            let seeded = {
                let conn = conn.clone();
                let data_dir = data_dir.clone();
                let session_id = session_id.clone();
                cx.background_executor()
                    .spawn(async move {
                        let client = Client::new(conn.clone())
                            .map_err(|e| format!("client handshake: {e}"))?;
                        let snap = client
                            .sessions()
                            .get(&session_id, GetOpts::default())
                            .map_err(|e| format!("get session {session_id}: {e}"))?;
                        crate::seed_disk(&conn, &session_id, &data_dir, &snap)
                    })
                    .await
            };
            seeded?;

            // Foreground: build the live session (poller + card + bridge).
            let client = Client::new(conn.clone()).map_err(|e| format!("client handshake: {e}"))?;
            store
                .update(cx, |store, cx| {
                    store
                        .spawn_live_session(&conn, &client, session_id.clone(), &data_dir, cx)
                        .map(|_card| ())
                })
                .map_err(|e| format!("store gone: {e:?}"))?
        })
    }
}
