//! Foreground-local input queue + off-fg forwarder onto the bounded `cmd_tx`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{Receiver, SendTimeoutError, Sender};

use super::worker::EngineCommand;

const CMD_SEND_TIMEOUT: Duration = Duration::from_millis(50);

enum ForwarderMsg {
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "enqueued via try_enqueue from handle in Task 5+")
    )]
    Cmd(EngineCommand),
    /// Drop all pending local commands (access downgrade).
    #[allow(dead_code, reason = "bump_access_epoch purge in Task 6")]
    Purge,
    /// Unblock `recv` during non-blocking stop / sever.
    Wake,
}

/// Off-foreground forwarder: unbounded FG-local queue → bounded `cmd_tx` with retry.
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
    pub(crate) fn spawn(cmd_tx: Sender<EngineCommand>) -> Self {
        let (local_tx, local_rx) = crossbeam_channel::unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let blocked_in_retry = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let (sever_done_tx, sever_done_rx) = crossbeam_channel::bounded(1);

        let stop_t = Arc::clone(&stop);
        let blocked_t = Arc::clone(&blocked_in_retry);
        let join = thread::Builder::new()
            .name("lens-terminal-input-forwarder".into())
            .spawn(move || {
                forward_loop(local_rx, cmd_tx, stop_t, blocked_t);
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
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "called from handle::enqueue_input in Task 5+")
    )]
    pub(crate) fn try_enqueue(&self, cmd: EngineCommand) -> Result<(), ()> {
        if self.stop.load(Ordering::Acquire) {
            return Err(());
        }
        self.local_tx.send(ForwarderMsg::Cmd(cmd)).map_err(|_| ())
    }

    /// Drop all commands still waiting in the local queue.
    #[allow(dead_code, reason = "bump_access_epoch in Task 6")]
    pub(crate) fn purge(&self) {
        if self.stop.load(Ordering::Acquire) {
            return;
        }
        let _ = self.local_tx.send(ForwarderMsg::Purge);
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

fn forward_loop(
    local_rx: Receiver<ForwarderMsg>,
    cmd_tx: Sender<EngineCommand>,
    stop: Arc<AtomicBool>,
    #[cfg_attr(not(test), allow(unused_variables))] blocked_in_retry: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Acquire) {
        let msg = match local_rx.recv() {
            Ok(m) => m,
            Err(_) => break,
        };

        match msg {
            ForwarderMsg::Purge => while local_rx.try_recv().is_ok() {},
            ForwarderMsg::Wake => {}
            ForwarderMsg::Cmd(cmd) => {
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
