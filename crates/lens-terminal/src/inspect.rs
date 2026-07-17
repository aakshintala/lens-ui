//! Convergence introspection snapshot (Slice 1d Task 9).

use lens_client::AttachInspect;
use serde::Serialize;

use crate::Lifecycle;

/// Point-in-time terminal tab snapshot for introspection tooling.
#[derive(Clone, Debug, Serialize)]
pub struct TerminalInspect {
    pub lifecycle: Lifecycle,
    pub output_gap: bool,
    pub bridge_alive: bool,
    pub input_enabled: bool,
    pub attach: Option<AttachInspect>,
    pub engine: Option<crate::EngineInspect>,
}
