//! Cloud / Web API transport (SPEC §4.2) — **deferred**.
//!
//! The device-side FW4 WebSocket URL, the enrolment/auth handshake, and the WS
//! framing of `RpcMessage` are firmware-baked and not present in the public
//! protobuf or any community project (SPEC §14 #1). Until a TLS-intercept
//! capture of a real FW4 device resolves them, this transport is a no-op.
//!
//! When unblocked, the implementation is small: open a `tokio-tungstenite`
//! TLS WebSocket to the resolved host, complete the device-registration
//! handshake, then bridge each binary WS message to a byte-frame channel and
//! hand it to [`crate::transport::serve_frames`] — exactly like the BLE
//! transport. A reconnect-with-backoff loop wraps the connection and bumps
//! `rod.cloud.reconnects.total`.

use tokio::sync::broadcast;
use tracing::warn;

use crate::config::Config;
use crate::rpc::dispatch::Dispatcher;
use crate::rpc::RpcMessage;

/// Entry point for the cloud transport task. Currently logs that it is disabled
/// and returns; see the module docs for what remains.
pub async fn run(
    cfg: &Config,
    _dispatcher: Dispatcher,
    _notif_rx: broadcast::Receiver<RpcMessage>,
) {
    let host = match cfg.cloud.server_env.as_str() {
        "staging" => "staging.handyfeeling.com",
        "custom" => cfg.cloud.custom_url.as_str(),
        _ => "www.handyfeeling.com",
    };
    warn!(
        server_env = %cfg.cloud.server_env,
        host,
        "cloud transport is not implemented (FW4 device socket URL/handshake unknown — SPEC §14 #1); \
         not connecting"
    );
}
