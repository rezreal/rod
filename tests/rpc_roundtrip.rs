//! End-to-end protobuf round-trips over the public `rpc` surface: encode an
//! `RpcMessage`, decode it back, and confirm the envelope + payload survive.

use rod::rpc::{self, request::Params, response::Result as Res, *};

#[test]
fn request_envelope_roundtrips() {
    let msg = RpcMessage::request(Request {
        params: Some(Params::RequestHampVelocitySet(RequestHampVelocitySet {
            velocity: 0.42,
        })),
        id: 123,
    });
    let bytes = rpc::encode_rpc(&msg);
    let back = rpc::decode_rpc(&bytes).expect("decode");

    assert_eq!(back.r#type, MessageType::Request as i32);
    match back.message {
        Some(rpc_message::Message::Request(r)) => {
            assert_eq!(r.id, 123);
            match r.params {
                Some(Params::RequestHampVelocitySet(v)) => assert_eq!(v.velocity, 0.42),
                other => panic!("wrong params: {other:?}"),
            }
        }
        other => panic!("wrong message: {other:?}"),
    }
}

#[test]
fn response_with_hsp_state_roundtrips() {
    let resp = Response {
        id: 7,
        result: Some(Res::ResponseHspStateGet(ResponseHspStateGet {
            state: Some(HspState {
                play_state: HspPlayState::HspStatePlaying as i32,
                points: 250,
                max_points: 4000,
                current_point: 12,
                current_time: 1200,
                r#loop: true,
                playback_rate: 1.5,
                first_point_time: 0,
                last_point_time: 60000,
                stream_id: 99,
                tail_point_stream_index: 249,
                tail_point_stream_index_threshold: 200,
                pause_on_starving: false,
            }),
        })),
        error: None,
    };
    let bytes = rpc::encode_rpc(&RpcMessage::response(resp));
    let back = rpc::decode_rpc(&bytes).expect("decode");
    match back.message {
        Some(rpc_message::Message::Response(r)) => {
            assert_eq!(r.id, 7);
            let Some(Res::ResponseHspStateGet(g)) = r.result else {
                panic!("missing result")
            };
            let s = g.state.unwrap();
            assert_eq!(s.points, 250);
            assert_eq!(s.stream_id, 99);
            assert!(s.r#loop);
            assert_eq!(s.playback_rate, 1.5);
        }
        other => panic!("wrong message: {other:?}"),
    }
}

#[test]
fn error_response_roundtrips() {
    let bytes = rpc::encode_rpc(&RpcMessage::response(Response::err(
        9,
        HandyErrorCodes::ErrorNotImplemented as i32,
        "nope",
    )));
    let back = rpc::decode_rpc(&bytes).unwrap();
    let Some(rpc_message::Message::Response(r)) = back.message else {
        panic!()
    };
    let e = r.error.unwrap();
    assert_eq!(e.code, HandyErrorCodes::ErrorNotImplemented as i32);
    assert_eq!(e.message, "nope");
}

#[test]
fn point_uses_8bit_x_scale() {
    // HSP points carry x in 0..255 (NOT 0..1 / 0..100).
    let p = Point { t: 500, x: 255 };
    let bytes = p.encode_to_vec_compat();
    let back = Point::decode_compat(&bytes);
    assert_eq!(back.x, 255);
    assert_eq!(back.t, 500);
}

// Small helpers so the test doesn't need to import `prost::Message` directly.
trait PointCompat {
    fn encode_to_vec_compat(&self) -> Vec<u8>;
    fn decode_compat(bytes: &[u8]) -> Self;
}
impl PointCompat for Point {
    fn encode_to_vec_compat(&self) -> Vec<u8> {
        use prost::Message;
        self.encode_to_vec()
    }
    fn decode_compat(bytes: &[u8]) -> Self {
        use prost::Message;
        Point::decode(bytes).unwrap()
    }
}
