//! Transports. Both BLE and the (deferred) cloud WebSocket carry the identical
//! `RpcMessage` envelope and differ only in framing (SPEC §4). The shared
//! decode → dispatch → encode loop lives here in [`serve_frames`]; each
//! transport only has to turn its wire into a byte-frame channel.

pub mod ble;
pub mod cloud;

use tokio::sync::{broadcast, mpsc};
use tracing::{debug, warn};

use crate::rpc::{self, dispatch::Dispatcher, rpc_message::Message, MessageType, RpcMessage};
use crate::telemetry::metrics;

/// Drive one connected client to completion: decode inbound frames into
/// `RpcMessage`s, dispatch each contained `Request`, send each `Response` back as
/// its own frame, and fan device notifications out to this client.
///
/// `rx_frames` yields complete inbound frames (one `RpcMessage` each, already
/// reassembled by the transport). `tx_frames` accepts complete outbound frames.
/// `notif_rx` is the device-wide notification broadcast.
///
/// Returns when the inbound channel closes (client disconnected) or the outbound
/// channel is gone.
pub async fn serve_frames(
    transport: &'static str,
    dispatcher: Dispatcher,
    mut rx_frames: mpsc::Receiver<Vec<u8>>,
    tx_frames: mpsc::Sender<Vec<u8>>,
    mut notif_rx: broadcast::Receiver<RpcMessage>,
) {
    metrics::transport_connected(transport, 1);
    loop {
        tokio::select! {
            frame = rx_frames.recv() => {
                let Some(bytes) = frame else { break };
                handle_inbound(transport, &dispatcher, &bytes, &tx_frames).await;
            }
            note = notif_rx.recv() => {
                match note {
                    Ok(msg) => {
                        if tx_frames.send(rpc::encode_rpc(&msg)).await.is_err() {
                            break;
                        }
                    }
                    // Lagged: we dropped some notifications; keep going.
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(transport, dropped = n, "notification subscriber lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => {}
                }
            }
        }
    }
    metrics::transport_connected(transport, -1);
    // Client gone: a transport loss must stop active motion (SPEC §10).
    dispatcher.stop_everything().await;
    debug!(
        transport,
        "transport connection closed; stopped active mode"
    );
}

/// Decode one inbound frame and dispatch every request it carries, sending each
/// response back as its own frame (responses are returned individually even for
/// a `Requests` bundle, per the protocol).
async fn handle_inbound(
    transport: &'static str,
    dispatcher: &Dispatcher,
    bytes: &[u8],
    tx_frames: &mpsc::Sender<Vec<u8>>,
) {
    let msg = match rpc::decode_rpc(bytes) {
        Ok(m) => m,
        Err(e) => {
            warn!(transport, error = %e, "failed to decode RpcMessage");
            return;
        }
    };

    let requests = match msg.message {
        Some(Message::Request(req)) => vec![req],
        Some(Message::Requests(reqs)) => reqs.requests,
        // Devices only receive REQUEST/REQUESTS; ignore anything else.
        _ => {
            warn!(transport, ty = msg.r#type, "ignoring non-request frame");
            return;
        }
    };

    // Diagnostic: an app request whose proto field we don't have decodes with
    // `params == None`. Dump the raw frame so the unknown field can be read.
    for req in &requests {
        if req.params.is_none() {
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            tracing::warn!(transport, id = req.id, %hex, "request with undecodable params");
        }
    }

    for req in requests {
        let label = rpc::dispatch::request_label(&req);
        let resp = dispatcher.handle_request(req).await;
        metrics::rpc_request(transport, label, resp.error.is_none());
        let out = RpcMessage {
            r#type: MessageType::Response as i32,
            message: Some(Message::Response(resp)),
        };
        if tx_frames.send(rpc::encode_rpc(&out)).await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::rpc::{request::Params, *};
    use crate::state::AppState;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn dispatcher() -> (Dispatcher, broadcast::Sender<RpcMessage>) {
        let cfg: Config =
            toml::from_str("[actuator]\nvariant='12inch'\n[actuator.limits]\n").unwrap();
        let state = Arc::new(RwLock::new(AppState::new("u".into(), 1)));
        let (cmd_tx, _cmd_rx) = mpsc::channel(16);
        let (notif_tx, _n) = broadcast::channel(16);
        let (hamp_tx, _h) = mpsc::channel(16);
        let (hsp_tx, _s) = mpsc::channel(16);
        let (drill_tx, _dr) = mpsc::channel(16);
        let (ramp_tx, _rm) = mpsc::channel(16);
        let (game_tx, _gm) = mpsc::channel(16);
        let (cycle_tx, _cy) = mpsc::channel(16);
        let (learn_tx, _ln) = mpsc::channel(16);
        let (pulse_tx, _pl) = mpsc::channel(16);
        let (impale_tx, _im) = mpsc::channel(16);
        let (coyote_tx, _co) = mpsc::channel(16);
        let (sensor_tx, _se) = mpsc::channel(16);
        let (plumb_tx, _pb) = mpsc::channel(16);
        let (surge_tx, _sg) = mpsc::channel(16);
        let (tide_tx, _td) = mpsc::channel(16);
        let (echo_tx, _ec) = mpsc::channel(16);
        let (trace_tx, _tr) = mpsc::channel(16);
        let (tempo_tx, _tp) = mpsc::channel(16);
        let modes = crate::modes::ModeControls { drill: drill_tx, ramp: ramp_tx, game: game_tx, cycle: cycle_tx, learn: learn_tx, pulse: pulse_tx, impale: impale_tx, coyote: coyote_tx, sensors: sensor_tx, plumb: plumb_tx, surge: surge_tx, tide: tide_tx, echo: echo_tx, trace: trace_tx, tempo: tempo_tx };
        let d = Dispatcher::new(state, cmd_tx, notif_tx.clone(), hamp_tx, hsp_tx, modes, &cfg);
        (d, notif_tx)
    }

    #[tokio::test]
    async fn request_frame_yields_response_frame() {
        let (d, notif_tx) = dispatcher();
        let (rx_in, rx_in_rx) = mpsc::channel(8);
        let (tx_out, mut tx_out_rx) = mpsc::channel(8);

        let h = tokio::spawn(serve_frames(
            "ble",
            d,
            rx_in_rx,
            tx_out,
            notif_tx.subscribe(),
        ));

        // Send a CapabilitiesGet request frame.
        let req = RpcMessage::request(Request {
            params: Some(Params::RequestCapabilitiesGet(RequestCapabilitiesGet {})),
            id: 5,
        });
        rx_in.send(rpc::encode_rpc(&req)).await.unwrap();

        let out = tx_out_rx.recv().await.unwrap();
        let decoded = rpc::decode_rpc(&out).unwrap();
        match decoded.message {
            Some(Message::Response(r)) => {
                assert_eq!(r.id, 5);
                assert!(matches!(
                    r.result,
                    Some(response::Result::ResponseCapabilitiesGet(_))
                ));
            }
            other => panic!("expected response, got {other:?}"),
        }

        drop(rx_in);
        let _ = h.await;
    }

    #[tokio::test]
    async fn notification_is_forwarded_to_client() {
        let (d, notif_tx) = dispatcher();
        let (_rx_in, rx_in_rx) = mpsc::channel(8);
        let (tx_out, mut tx_out_rx) = mpsc::channel(8);
        let h = tokio::spawn(serve_frames(
            "cloud",
            d,
            rx_in_rx,
            tx_out,
            notif_tx.subscribe(),
        ));

        // Broadcast a notification; it should arrive as an outbound frame.
        let note = RpcMessage::notification(Notification {
            id: 0,
            notification: Some(notification::Notification::NotificationError(
                NotificationError {
                    code: 42,
                    message: "boom".into(),
                },
            )),
        });
        notif_tx.send(note).unwrap();

        let out = tx_out_rx.recv().await.unwrap();
        let decoded = rpc::decode_rpc(&out).unwrap();
        assert!(matches!(decoded.message, Some(Message::Notification(_))));
        drop(_rx_in);
        let _ = h.await;
    }
}
