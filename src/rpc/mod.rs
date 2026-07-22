//! Handy RPC protocol types, generated from the vendored `.proto` files by
//! `prost-build` (see `build.rs`). Everything in package `hdy_rpc` is re-exported
//! here so the rest of the crate can `use crate::rpc::*`.

#![allow(clippy::all)]
include!(concat!(env!("OUT_DIR"), "/hdy_rpc.rs"));

pub mod dispatch;

use prost::Message;

/// Decode a length-delimited-free `RpcMessage` from a transport frame.
///
/// Both BLE (GATT TX/RX) and the cloud WebSocket carry one bare `RpcMessage`
/// per frame (no extra length prefix — the transport already delimits it), so
/// this is a plain protobuf decode.
pub fn decode_rpc(bytes: &[u8]) -> Result<RpcMessage, prost::DecodeError> {
    RpcMessage::decode(bytes)
}

/// Encode an `RpcMessage` into the bytes of a single transport frame.
pub fn encode_rpc(msg: &RpcMessage) -> Vec<u8> {
    msg.encode_to_vec()
}

impl RpcMessage {
    /// Wrap a single `Response` in an `RpcMessage` envelope.
    pub fn response(resp: Response) -> Self {
        RpcMessage {
            r#type: MessageType::Response as i32,
            message: Some(rpc_message::Message::Response(resp)),
        }
    }

    /// Wrap a `Notification` in an `RpcMessage` envelope.
    pub fn notification(note: Notification) -> Self {
        RpcMessage {
            r#type: MessageType::Notification as i32,
            message: Some(rpc_message::Message::Notification(note)),
        }
    }

    /// Wrap a single `Request` in an `RpcMessage` envelope (used by tests).
    pub fn request(req: Request) -> Self {
        RpcMessage {
            r#type: MessageType::Request as i32,
            message: Some(rpc_message::Message::Request(req)),
        }
    }
}

impl Response {
    /// A blank response echoing a request id, with no result and no error.
    pub fn blank(id: u32) -> Self {
        Response {
            id,
            result: None,
            error: None,
        }
    }

    /// An error response echoing a request id.
    pub fn err(id: u32, code: i32, message: impl Into<String>) -> Self {
        Response {
            id,
            result: None,
            error: Some(Error {
                code,
                message: message.into(),
                data: String::new(),
            }),
        }
    }
}
