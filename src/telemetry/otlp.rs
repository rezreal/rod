//! OTLP pipeline init, gated entirely by the standard OTel environment
//! variables (SPEC §11.1). No TOML config.
//!
//! Behaviour:
//! * `tracing-subscriber` (stderr) is always installed.
//! * If `OTEL_SDK_DISABLED` is not `true` **and** an OTLP endpoint is configured
//!   (`OTEL_EXPORTER_OTLP_ENDPOINT` or a signal-specific
//!   `OTEL_EXPORTER_OTLP_{LOGS,METRICS,TRACES}_ENDPOINT`), the logs+metrics+
//!   traces pipeline is initialised. Otherwise the SDK stays off — zero exporter
//!   threads, zero overhead.
//!
//! Protocol selection honours `OTEL_EXPORTER_OTLP_PROTOCOL`
//! (`grpc` | `http/protobuf`); endpoints/headers/etc. are read by the SDK from
//! their standard variables.

use anyhow::Context;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_sdk::logs::LoggerProvider;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::TracerProvider as SdkTracerProvider;
use opentelemetry_sdk::{runtime, Resource};
use tracing_subscriber::prelude::*;

/// RAII guard that flushes and shuts down the OTel providers on drop.
pub struct TelemetryGuard {
    providers: Option<Providers>,
}

struct Providers {
    meter: SdkMeterProvider,
    tracer: SdkTracerProvider,
    logger: LoggerProvider,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if let Some(p) = self.providers.take() {
            // Best-effort flush on shutdown.
            let _ = p.meter.shutdown();
            let _ = p.tracer.shutdown();
            let _ = p.logger.shutdown();
        }
    }
}

/// True when the OTLP pipeline should be enabled per the env vars.
fn otlp_enabled() -> bool {
    if std::env::var("OTEL_SDK_DISABLED").as_deref() == Ok("true") {
        return false;
    }
    [
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
    ]
    .iter()
    .any(|k| std::env::var(k).is_ok())
}

/// `true` for gRPC (tonic), `false` for HTTP/protobuf.
fn use_grpc() -> bool {
    match std::env::var("OTEL_EXPORTER_OTLP_PROTOCOL").as_deref() {
        Ok("http/protobuf") | Ok("http/json") => false,
        _ => true, // default: grpc
    }
}

fn resource() -> Resource {
    // `Resource::default()` already merges OTEL_SERVICE_NAME / OTEL_RESOURCE_ATTRIBUTES.
    // Ensure a friendly default service name when none is configured.
    let base = Resource::default();
    if std::env::var("OTEL_SERVICE_NAME").is_ok() {
        base
    } else {
        Resource::new([opentelemetry::KeyValue::new(
            opentelemetry_semantic_conventions::resource::SERVICE_NAME,
            "rod",
        )])
        .merge(&base)
    }
}

fn metric_exporter(grpc: bool) -> anyhow::Result<opentelemetry_otlp::MetricExporter> {
    let b = opentelemetry_otlp::MetricExporter::builder();
    Ok(if grpc {
        b.with_tonic().build()?
    } else {
        b.with_http().build()?
    })
}

fn span_exporter(grpc: bool) -> anyhow::Result<opentelemetry_otlp::SpanExporter> {
    let b = opentelemetry_otlp::SpanExporter::builder();
    Ok(if grpc {
        b.with_tonic().build()?
    } else {
        b.with_http().build()?
    })
}

fn log_exporter(grpc: bool) -> anyhow::Result<opentelemetry_otlp::LogExporter> {
    let b = opentelemetry_otlp::LogExporter::builder();
    Ok(if grpc {
        b.with_tonic().build()?
    } else {
        b.with_http().build()?
    })
}

/// Initialise telemetry. Always sets up stderr logging; conditionally wires OTLP.
pub fn init() -> anyhow::Result<TelemetryGuard> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let fmt_layer = tracing_subscriber::fmt::layer();

    if !otlp_enabled() {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .try_init()
            .ok();
        tracing::info!("telemetry: OTLP disabled (no endpoint / OTEL_SDK_DISABLED); stderr only");
        return Ok(TelemetryGuard { providers: None });
    }

    let grpc = use_grpc();
    let res = resource();

    // Metrics: periodic-export meter provider.
    let reader = PeriodicReader::builder(
        metric_exporter(grpc).context("metric exporter")?,
        runtime::Tokio,
    )
    .build();
    let meter = SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(res.clone())
        .build();

    // Traces: batch span exporter.
    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(
            span_exporter(grpc).context("span exporter")?,
            runtime::Tokio,
        )
        .with_resource(res.clone())
        .build();
    let tracer = tracer_provider.tracer("rod");

    // Logs: batch log exporter, bridged from `tracing` events.
    let logger_provider = LoggerProvider::builder()
        .with_batch_exporter(log_exporter(grpc).context("log exporter")?, runtime::Tokio)
        .with_resource(res)
        .build();
    let log_bridge = OpenTelemetryTracingBridge::new(&logger_provider);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .with(log_bridge)
        .try_init()
        .ok();

    opentelemetry::global::set_meter_provider(meter.clone());
    opentelemetry::global::set_tracer_provider(tracer_provider.clone());

    tracing::info!(
        protocol = if grpc { "grpc" } else { "http/protobuf" },
        "telemetry: OTLP pipeline initialised (logs + metrics + traces)"
    );

    Ok(TelemetryGuard {
        providers: Some(Providers {
            meter,
            tracer: tracer_provider,
            logger: logger_provider,
        }),
    })
}
