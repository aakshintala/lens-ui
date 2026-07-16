use std::time::Duration;

use crate::client::Client;
use crate::error::Result;
use crate::ids::{SessionId, TerminalId};

use super::close::CloseCause;
use super::wire::{WsInbound, WsOutbound};

#[derive(Clone, Copy, Debug)]
pub struct AttachOptions {
    pub read_only: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct AttachInspect {
    pub connected: bool,
    pub inbound_len: usize,
    pub inbound_cap: usize,
    pub outbound_len: usize,
    pub outbound_cap: usize,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub last_close: Option<CloseCause>,
    pub recent: Vec<InspectEvent>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct InspectEvent {
    pub kind: &'static str,
}

pub struct AttachHandle {
    pub inbound: crossbeam_channel::Receiver<WsInbound>,
    pub outbound: crossbeam_channel::Sender<WsOutbound>,
}

impl AttachHandle {
    pub fn close(self) {}

    pub fn inspect(&self) -> AttachInspect {
        AttachInspect {
            connected: false,
            inbound_len: 0,
            inbound_cap: 0,
            outbound_len: 0,
            outbound_cap: 0,
            bytes_in: 0,
            bytes_out: 0,
            last_close: None,
            recent: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Backoff {
    attempt: u32,
}

impl Backoff {
    pub fn next_delay(&mut self) -> Duration {
        let _ = self.attempt;
        todo!("Task 5")
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

pub fn attach(
    _client: &Client,
    _session: &SessionId,
    _tid: &TerminalId,
    _opts: AttachOptions,
) -> Result<AttachHandle> {
    todo!("Task 5")
}
