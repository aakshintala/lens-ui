//! Foreground-local input queue + off-fg forwarder onto the bounded `cmd_tx`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{Receiver, SendTimeoutError, Sender};

use super::command::InputAck;
use super::worker::EngineCommand;

const CMD_SEND_TIMEOUT: Duration = Duration::from_millis(50);

enum ForwarderMsg {
    Cmd(EngineCommand),
    /// Unblock `recv` during non-blocking stop / sever.
    Wake,
}

/// Off-foreground forwarder: unbounded FG-local queue → bounded `cmd_tx` with retry.
///
/// Access downgrade revoke is epoch-gated at forward time: each `Cmd` is dropped if its
/// stamped epoch is strictly less than the shared [`access_epoch`](AtomicU64). The worker's
/// final-egress epoch recheck (Slice 2a Task 6) is the authoritative second layer for
/// commands already mid-retry on `cmd_tx`.
pub(crate) struct InputForwarder {
    local_tx: Sender<ForwarderMsg>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "test barrier — read via blocked_in_retry()")
    )]
    blocked_in_retry: Arc<AtomicBool>,
    #[cfg(test)]
    sever_done_rx: Option<Receiver<()>>,
}

impl InputForwarder {
    pub(crate) fn spawn(cmd_tx: Sender<EngineCommand>, access_epoch: Arc<AtomicU64>) -> Self {
        let (local_tx, local_rx) = crossbeam_channel::unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let blocked_in_retry = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let (sever_done_tx, sever_done_rx) = crossbeam_channel::bounded(1);

        let stop_t = Arc::clone(&stop);
        let blocked_t = Arc::clone(&blocked_in_retry);
        let epoch_t = Arc::clone(&access_epoch);
        let join = thread::Builder::new()
            .name("lens-terminal-input-forwarder".into())
            .spawn(move || {
                forward_loop(local_rx, cmd_tx, stop_t, blocked_t, epoch_t);
                #[cfg(test)]
                let _ = sever_done_tx.send(());
            })
            .expect("spawn input forwarder");

        Self {
            local_tx,
            stop,
            join: Some(join),
            blocked_in_retry,
            #[cfg(test)]
            sever_done_rx: Some(sever_done_rx),
        }
    }

    /// Never blocks the caller — pushes into the unbounded local queue.
    pub(crate) fn try_enqueue(&self, cmd: EngineCommand) -> Result<(), ()> {
        if self.stop.load(Ordering::Acquire) {
            return Err(());
        }
        self.local_tx.send(ForwarderMsg::Cmd(cmd)).map_err(|_| ())
    }

    /// Set `stop`, wake the thread, and block until it exits.
    pub(crate) fn sever_and_join(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.local_tx.send(ForwarderMsg::Wake);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }

    /// Non-blocking stop for `EngineHandle::Drop` — never joins.
    pub(crate) fn signal_stop_nonblocking(&self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.local_tx.try_send(ForwarderMsg::Wake);
    }

    #[cfg(test)]
    pub(crate) fn blocked_in_retry(&self) -> &Arc<AtomicBool> {
        &self.blocked_in_retry
    }

    #[cfg(test)]
    pub(crate) fn take_sever_done_rx(&mut self) -> Option<Receiver<()>> {
        self.sever_done_rx.take()
    }
}

/// Honest revoke ack for stale Key/Paste commands — never leave the caller hanging.
fn send_stale_revoke_ack(cmd: EngineCommand) {
    let ack_tx = match cmd {
        EngineCommand::Key(mut input) => input.ack.take(),
        EngineCommand::Paste(mut input) => input.ack.take(),
        _ => None,
    };
    if let Some(tx) = ack_tx {
        let _ = tx.try_send(InputAck {
            encoded: Vec::new(),
            accepted: false,
        });
    }
}

/// True when a stamped command belongs to a prior access epoch and must not be forwarded.
fn is_stale(cmd: &EngineCommand, current_epoch: u64) -> bool {
    match cmd {
        EngineCommand::Key(k) => k.access_epoch < current_epoch,
        EngineCommand::Paste(p) => p.access_epoch < current_epoch,
        EngineCommand::Focus { access_epoch, .. } => *access_epoch < current_epoch,
        _ => false,
    }
}

fn forward_loop(
    local_rx: Receiver<ForwarderMsg>,
    cmd_tx: Sender<EngineCommand>,
    stop: Arc<AtomicBool>,
    #[cfg_attr(not(test), allow(unused_variables))] blocked_in_retry: Arc<AtomicBool>,
    access_epoch: Arc<AtomicU64>,
) {
    while !stop.load(Ordering::Acquire) {
        let msg = match local_rx.recv() {
            Ok(m) => m,
            Err(_) => break,
        };

        match msg {
            ForwarderMsg::Wake => {}
            ForwarderMsg::Cmd(cmd) => {
                if is_stale(&cmd, access_epoch.load(Ordering::Acquire)) {
                    send_stale_revoke_ack(cmd);
                    continue;
                }
                let mut pending = cmd;
                loop {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    match cmd_tx.send_timeout(pending, CMD_SEND_TIMEOUT) {
                        Ok(()) => break,
                        Err(SendTimeoutError::Timeout(p)) => {
                            #[cfg(test)]
                            blocked_in_retry.store(true, Ordering::Release);
                            pending = p;
                        }
                        Err(SendTimeoutError::Disconnected(_)) => return,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::engine::command::{KeyAction, KeyInput, KeyMods, LensKey, PasteInput, ScrollDelta};

    fn key_with_epoch(epoch: u64) -> EngineCommand {
        EngineCommand::Key(KeyInput {
            action: KeyAction::Press,
            key: LensKey::A,
            mods: KeyMods::default(),
            utf8: Some("a".into()),
            composing: false,
            access_epoch: epoch,
            ack: None,
        })
    }

    fn paste_with_epoch(epoch: u64) -> EngineCommand {
        EngineCommand::Paste(PasteInput {
            bytes: b"ab".to_vec(),
            access_epoch: epoch,
            ack: None,
        })
    }

    #[test]
    fn is_stale_drops_prior_epoch_key_and_focus_not_scroll() {
        assert!(is_stale(&key_with_epoch(0), 1));
        assert!(!is_stale(&key_with_epoch(1), 1));
        assert!(is_stale(&paste_with_epoch(0), 1));
        assert!(!is_stale(&paste_with_epoch(1), 1));
        assert!(is_stale(
            &EngineCommand::Focus {
                focused: true,
                report: false,
                access_epoch: 0,
            },
            2
        ));
        assert!(!is_stale(
            &EngineCommand::LocalScroll(ScrollDelta::Lines(1)),
            u64::MAX
        ));
    }

    #[test]
    fn stale_key_with_ack_gets_revoke_ack_before_drop() {
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        send_stale_revoke_ack(EngineCommand::Key(KeyInput {
            action: KeyAction::Press,
            key: LensKey::Z,
            mods: KeyMods::default(),
            utf8: Some("z".into()),
            composing: false,
            access_epoch: 0,
            ack: Some(ack_tx),
        }));
        let ack = ack_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("stale ack");
        assert!(!ack.accepted);
        assert!(ack.encoded.is_empty());
    }

    #[test]
    fn stale_paste_with_ack_gets_revoke_ack_before_drop() {
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        send_stale_revoke_ack(EngineCommand::Paste(PasteInput {
            bytes: b"ab".to_vec(),
            access_epoch: 0,
            ack: Some(ack_tx),
        }));
        let ack = ack_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("stale ack");
        assert!(!ack.accepted);
        assert!(ack.encoded.is_empty());
    }
}
