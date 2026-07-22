//! Hardware hand-switch watcher.
//!
//! The IAI controller's hand/palm switch (DIPM bit 0) is surfaced as
//! [`AppState::hand_switch`](crate::state::AppState) by the Modbus status poll.
//! This task watches that bit for press/release edges and drives whichever
//! program is currently active exactly as that program's on-screen button would
//! — so the physical switch and the web button are interchangeable, with one
//! exception: a game's triple-tap ready gesture only accepts hardware presses
//! (see [`crate::modes::GameControl::HardwareTap`]).
//!
//! The per-mode routing lives in [`Dispatcher::hand_switch`] so it can reach
//! every mode's control channel (and the dispatcher's own start/stop paths).

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::info;

use crate::rpc::dispatch::Dispatcher;
use crate::state::AppState;

/// A transition of the hand switch, fed to the dispatcher each tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandEdge {
    /// Rising edge — the switch was just pressed.
    Press,
    /// Still held this tick (used to resend deadman heartbeats).
    Hold,
    /// Falling edge — the switch was just released.
    Release,
}

/// Poll cadence. The DIPM bit only refreshes on the Modbus status poll (~80 ms),
/// so a faster tick here just bounds the added latency without spamming reads.
const TICK: Duration = Duration::from_millis(30);

/// Watch the hand-switch bit and dispatch edges to the active program forever.
pub async fn run(state: Arc<RwLock<AppState>>, dispatcher: Dispatcher) {
    info!("hand-switch watcher running");
    let mut tick = interval(TICK);
    let mut was_pressed = false;
    loop {
        tick.tick().await;
        let pressed = state.read().await.hand_switch;
        let edge = match (was_pressed, pressed) {
            (false, true) => Some(HandEdge::Press),
            (true, true) => Some(HandEdge::Hold),
            (true, false) => Some(HandEdge::Release),
            (false, false) => None,
        };
        was_pressed = pressed;
        if let Some(e) = edge {
            dispatcher.hand_switch(e).await;
        }
    }
}
