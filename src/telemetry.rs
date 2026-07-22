//! Observability (SPEC §11). Telemetry is opt-in and driven entirely by the
//! standard OTel environment variables — there is no TOML config.
//!
//! * [`init`] sets up `tracing-subscriber` (always, to stderr) and, when an
//!   `OTEL_EXPORTER_OTLP_*_ENDPOINT` is configured and the SDK is not disabled,
//!   the OTLP logs+metrics+traces pipeline.
//! * [`metrics`] holds typed helpers over instruments created once from the
//!   global meter. They are cheap no-ops until a meter provider is installed,
//!   so the rest of the code can call them unconditionally.

use std::sync::LazyLock;

use opentelemetry::metrics::{Counter, Histogram, Meter, UpDownCounter};
use opentelemetry::KeyValue;

mod otlp;
pub use otlp::{init, TelemetryGuard};

/// All instruments, created once from `global::meter("rod")`.
struct Instruments {
    moves_total: Counter<u64>,
    move_distance: Histogram<f64>,
    hsp_points_processed: Counter<u64>,
    hsp_points_added: Counter<u64>,
    hsp_starving_total: Counter<u64>,
    hsp_loops_total: Counter<u64>,
    hamp_strokes_total: Counter<u64>,
    hamp_cycle_time: Histogram<f64>,
    hdsp_commands_total: Counter<u64>,
    rpc_requests_total: Counter<u64>,
    rpc_dispatch_latency: Histogram<f64>,
    transport_connected: UpDownCounter<i64>,
    cloud_reconnects_total: Counter<u64>,
    modbus_txn_total: Counter<u64>,
    modbus_rtt: Histogram<f64>,
    modbus_errors_total: Counter<u64>,
    homing_total: Counter<u64>,
    homing_duration: Histogram<f64>,
    alarms_total: Counter<u64>,
}

impl Instruments {
    fn new(m: &Meter) -> Self {
        Instruments {
            moves_total: m.u64_counter("rod.moves.total").build(),
            move_distance: m.f64_histogram("rod.move.distance").with_unit("mm").build(),
            hsp_points_processed: m.u64_counter("rod.hsp.points_processed").build(),
            hsp_points_added: m.u64_counter("rod.hsp.points_added").build(),
            hsp_starving_total: m.u64_counter("rod.hsp.starving.total").build(),
            hsp_loops_total: m.u64_counter("rod.hsp.loops.total").build(),
            hamp_strokes_total: m.u64_counter("rod.hamp.strokes.total").build(),
            hamp_cycle_time: m
                .f64_histogram("rod.hamp.cycle_time")
                .with_unit("ms")
                .build(),
            hdsp_commands_total: m.u64_counter("rod.hdsp.commands.total").build(),
            rpc_requests_total: m.u64_counter("rod.rpc.requests.total").build(),
            rpc_dispatch_latency: m
                .f64_histogram("rod.rpc.dispatch_latency")
                .with_unit("ms")
                .build(),
            transport_connected: m.i64_up_down_counter("rod.transport.connected").build(),
            cloud_reconnects_total: m.u64_counter("rod.cloud.reconnects.total").build(),
            modbus_txn_total: m.u64_counter("rod.modbus.txn.total").build(),
            modbus_rtt: m.f64_histogram("rod.modbus.rtt").with_unit("ms").build(),
            modbus_errors_total: m.u64_counter("rod.modbus.errors.total").build(),
            homing_total: m.u64_counter("rod.homing.total").build(),
            homing_duration: m
                .f64_histogram("rod.homing.duration")
                .with_unit("ms")
                .build(),
            alarms_total: m.u64_counter("rod.alarms.total").build(),
        }
    }
}

static INSTRUMENTS: LazyLock<Instruments> =
    LazyLock::new(|| Instruments::new(&opentelemetry::global::meter("rod")));

/// Typed metric helpers. See SPEC §11.2. Each maps to one instrument; attribute
/// cardinality is kept bounded per §11.4 (no raw positions/timestamps/keys).
pub mod metrics {
    use super::*;

    pub fn move_issued(mode: &'static str, distance_mm: f64) {
        INSTRUMENTS
            .moves_total
            .add(1, &[KeyValue::new("mode", mode)]);
        INSTRUMENTS.move_distance.record(distance_mm.abs(), &[]);
    }

    pub fn hsp_points_processed(n: u64, stream_id: u32) {
        INSTRUMENTS
            .hsp_points_processed
            .add(n, &[KeyValue::new("stream_id", stream_id as i64)]);
    }

    pub fn hsp_points_added(n: u64) {
        INSTRUMENTS.hsp_points_added.add(n, &[]);
    }

    pub fn hsp_starving() {
        INSTRUMENTS.hsp_starving_total.add(1, &[]);
    }

    pub fn hsp_loop() {
        INSTRUMENTS.hsp_loops_total.add(1, &[]);
    }

    pub fn hamp_stroke(direction: bool, cycle_ms: f64) {
        INSTRUMENTS.hamp_strokes_total.add(
            1,
            &[KeyValue::new(
                "direction",
                if direction { "in" } else { "out" },
            )],
        );
        INSTRUMENTS.hamp_cycle_time.record(cycle_ms, &[]);
    }

    pub fn hdsp_command() {
        INSTRUMENTS.hdsp_commands_total.add(1, &[]);
    }

    pub fn rpc_request(transport: &'static str, request: &'static str, ok: bool) {
        INSTRUMENTS.rpc_requests_total.add(
            1,
            &[
                KeyValue::new("transport", transport),
                KeyValue::new("request", request),
                KeyValue::new("result", if ok { "ok" } else { "error" }),
            ],
        );
    }

    pub fn rpc_dispatch_latency_ms(ms: f64) {
        INSTRUMENTS.rpc_dispatch_latency.record(ms, &[]);
    }

    pub fn transport_connected(transport: &'static str, delta: i64) {
        INSTRUMENTS
            .transport_connected
            .add(delta, &[KeyValue::new("transport", transport)]);
    }

    pub fn cloud_reconnect() {
        INSTRUMENTS.cloud_reconnects_total.add(1, &[]);
    }

    pub fn modbus_txn(func: &'static str, result: &'static str, rtt_ms: f64) {
        INSTRUMENTS.modbus_txn_total.add(
            1,
            &[KeyValue::new("fn", func), KeyValue::new("result", result)],
        );
        INSTRUMENTS.modbus_rtt.record(rtt_ms, &[]);
        if result != "ok" {
            INSTRUMENTS
                .modbus_errors_total
                .add(1, &[KeyValue::new("kind", result)]);
        }
    }

    pub fn homing(result: &'static str, duration_ms: f64) {
        INSTRUMENTS
            .homing_total
            .add(1, &[KeyValue::new("result", result)]);
        INSTRUMENTS.homing_duration.record(duration_ms, &[]);
    }

    pub fn alarm(almc: u16) {
        INSTRUMENTS
            .alarms_total
            .add(1, &[KeyValue::new("almc", almc as i64)]);
    }
}
