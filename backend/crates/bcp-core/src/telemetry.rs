//! Logging, tracing (OTLP, optional) and Prometheus metrics initialisation.

use std::net::SocketAddr;

use anyhow::{Context, Result};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry::KeyValue;
use opentelemetry_sdk::trace::TracerProvider;
use opentelemetry_sdk::Resource;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

use crate::config::{env_opt, LogFormat};

/// Flushes pending spans on drop.
pub struct TelemetryGuard {
    provider: Option<TracerProvider>,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if let Some(p) = self.provider.take() {
            if let Err(e) = p.shutdown() {
                eprintln!("failed to flush traces: {e}");
            }
        }
    }
}

pub fn init(service_name: &str, format: LogFormat) -> Result<TelemetryGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn"));

    let fmt_layer = match format {
        LogFormat::Json => tracing_subscriber::fmt::layer()
            .json()
            .flatten_event(true)
            .with_current_span(true)
            .with_span_list(false)
            .with_target(true)
            .boxed(),
        LogFormat::Pretty => tracing_subscriber::fmt::layer().with_target(true).boxed(),
    };

    let (otel_layer, provider) = if env_opt("OTEL_EXPORTER_OTLP_ENDPOINT").is_some() {
        let exporter =
            opentelemetry_otlp::SpanExporter::builder().with_tonic().build().context("building OTLP span exporter")?;
        let service = env_opt("OTEL_SERVICE_NAME").unwrap_or_else(|| service_name.to_owned());
        let provider = TracerProvider::builder()
            .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
            .with_resource(Resource::new(vec![KeyValue::new("service.name", service)]))
            .build();
        let tracer = provider.tracer("bcp");
        (Some(tracing_opentelemetry::layer().with_tracer(tracer)), Some(provider))
    } else {
        (None, None)
    };

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(otel_layer)
        .try_init()
        .context("installing tracing subscriber")?;

    Ok(TelemetryGuard { provider })
}

/// Starts the Prometheus exporter on its own listener (`/metrics`).
pub fn init_metrics(addr: SocketAddr) -> Result<()> {
    metrics_exporter_prometheus::PrometheusBuilder::new()
        .with_http_listener(addr)
        .set_buckets(&[0.005, 0.025, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 15.0, 60.0, 180.0, 600.0, 1800.0])
        .context("configuring histogram buckets")?
        .install()
        .context("starting Prometheus exporter")
}
