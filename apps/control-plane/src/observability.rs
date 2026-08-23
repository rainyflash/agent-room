use opentelemetry::{global, trace::TracerProvider as _};
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::{Resource, propagation::TraceContextPropagator, trace::SdkTracerProvider};
use thiserror::Error;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{SERVICE_NAME, config::ObservabilityConfig};

pub(crate) struct Observability {
    tracer_provider: Option<SdkTracerProvider>,
    shutdown_timeout: std::time::Duration,
}

impl Observability {
    pub(crate) fn install(config: &ObservabilityConfig) -> Result<Self, ObservabilityError> {
        global::set_text_map_propagator(TraceContextPropagator::new());
        let filter = EnvFilter::try_new(&config.log_filter)
            .map_err(|_| ObservabilityError::InvalidFilter)?;

        if let Some(endpoint) = &config.otlp_traces_endpoint {
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_protocol(Protocol::HttpBinary)
                .with_endpoint(endpoint.to_string())
                .with_timeout(config.export_timeout)
                .build()
                .map_err(|_| ObservabilityError::Exporter)?;
            let provider = SdkTracerProvider::builder()
                .with_resource(Resource::builder().with_service_name(SERVICE_NAME).build())
                .with_batch_exporter(exporter)
                .build();
            let tracer = provider.tracer(SERVICE_NAME);
            let subscriber = tracing_subscriber::registry()
                .with(filter)
                .with(
                    tracing_subscriber::fmt::layer()
                        .json()
                        .with_current_span(true)
                        .with_span_list(false),
                )
                .with(tracing_opentelemetry::layer().with_tracer(tracer));
            if subscriber.try_init().is_err() {
                let _ = provider.shutdown_with_timeout(config.export_timeout);
                return Err(ObservabilityError::Subscriber);
            }
            global::set_tracer_provider(provider.clone());

            return Ok(Self {
                tracer_provider: Some(provider),
                shutdown_timeout: config.export_timeout,
            });
        }

        tracing_subscriber::registry()
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_current_span(true)
                    .with_span_list(false),
            )
            .try_init()
            .map_err(|_| ObservabilityError::Subscriber)?;

        Ok(Self {
            tracer_provider: None,
            shutdown_timeout: config.export_timeout,
        })
    }

    pub(crate) fn shutdown(self) {
        if let Some(provider) = self.tracer_provider
            && provider
                .shutdown_with_timeout(self.shutdown_timeout)
                .is_err()
        {
            eprintln!("OpenTelemetry 关闭失败：telemetry.shutdown_failed");
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum ObservabilityError {
    #[error("日志过滤规则无效")]
    InvalidFilter,
    #[error("OpenTelemetry 导出器初始化失败")]
    Exporter,
    #[error("全局遥测订阅器初始化失败")]
    Subscriber,
}
