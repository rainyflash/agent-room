use std::{str::FromStr, time::Instant};

use axum::{
    extract::Request,
    http::{HeaderMap, HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};
use opentelemetry::{
    global,
    propagation::{Extractor, Injector},
};
use tracing::{Instrument, Span};
use tracing_opentelemetry::{OpenTelemetrySpanExt, SetParentError};
use uuid::Uuid;

pub(crate) const CORRELATION_ID_HEADER: &str = "x-correlation-id";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CorrelationId(Uuid);

impl CorrelationId {
    pub(crate) fn from_headers(headers: &HeaderMap) -> Self {
        let supplied = headers
            .get(CORRELATION_ID_HEADER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| Uuid::parse_str(value).ok())
            .filter(|value| !value.is_nil());

        Self(supplied.unwrap_or_else(Uuid::now_v7))
    }

    pub(crate) fn as_uuid(self) -> Uuid {
        self.0
    }
}

pub(crate) async fn attach(mut request: Request, next: Next) -> Response {
    let correlation_id = CorrelationId::from_headers(request.headers());
    let parent_context = global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(request.headers()))
    });
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let span = tracing::info_span!(
        "http.request",
        otel.kind = "server",
        http.request.method = %method,
        url.path = %path,
        correlation.id = %correlation_id.as_uuid()
    );
    link_upstream(&span, parent_context);

    request.extensions_mut().insert(correlation_id);
    let header_value = HeaderValue::from_str(&correlation_id.as_uuid().to_string()).ok();
    if let Some(value) = header_value.clone() {
        request.headers_mut().insert(CORRELATION_ID_HEADER, value);
    } else {
        tracing::error!(
            code = "correlation.header_encode_failed",
            "关联 ID 无法编码为响应头"
        );
    }

    let started_at = Instant::now();
    let mut response = async move { next.run(request).await }
        .instrument(span.clone())
        .await;
    let latency_millis = u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::info!(
        parent: &span,
        http.response.status_code = response.status().as_u16(),
        latency_ms = latency_millis,
        "请求处理完成"
    );

    if let Some(value) = header_value {
        response.headers_mut().insert(CORRELATION_ID_HEADER, value);
    }
    response
}

/// 把这次请求挂到上游的追踪上。
///
/// 没配追踪导出（`AGENT_ROOM_OTLP_TRACES_ENDPOINT`）时根本没有 OpenTelemetry 层，span 被日志过滤掉时
/// 也无从挂接。这两种都是正常配置：以前每个请求都为此记一条告警，生产上每小时上千条，也不是上游的错。
/// 只有 span 已经开始这种代码问题才告警。
fn link_upstream(span: &Span, parent: opentelemetry::Context) {
    match span.set_parent(parent) {
        Ok(()) | Err(SetParentError::LayerNotFound | SetParentError::SpanDisabled) => {}
        Err(error @ SetParentError::AlreadyStarted) => {
            tracing::warn!(code = "telemetry.invalid_parent", %error, "无法关联上游追踪上下文");
        }
    }
}

pub(crate) fn outbound_headers(correlation_id: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(correlation_id) {
        headers.insert(CORRELATION_ID_HEADER, value);
    } else {
        tracing::warn!(code = "correlation.outbound_invalid", "拒绝传播无效关联 ID");
    }

    let context = Span::current().context();
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&context, &mut HeaderInjector(&mut headers));
    });
    headers
}

struct HeaderExtractor<'a>(&'a HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(HeaderName::as_str).collect()
    }
}

struct HeaderInjector<'a>(&'a mut HeaderMap);

impl Injector for HeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        let header_name = HeaderName::from_str(key);
        let header_value = HeaderValue::from_str(&value);
        if let (Ok(name), Ok(value)) = (header_name, header_value) {
            self.0.insert(name, value);
        } else {
            tracing::warn!(
                code = "telemetry.header_injection_failed",
                "追踪传播头编码失败"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use axum::http::{HeaderMap, HeaderValue};
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use tracing_subscriber::layer::SubscriberExt as _;
    use uuid::{Uuid, Version};

    use super::{CORRELATION_ID_HEADER, CorrelationId, link_upstream};

    struct 告警计数(Arc<AtomicUsize>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for 告警计数 {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _context: tracing_subscriber::layer::Context<'_, S>,
        ) {
            if *event.metadata().level() == tracing::Level::WARN {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    #[test]
    fn 没配追踪导出时挂不上上游也不告警() {
        let warnings = Arc::new(AtomicUsize::new(0));
        let subscriber = tracing_subscriber::registry().with(告警计数(warnings.clone()));
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("http.request");
            link_upstream(&span, opentelemetry::Context::new());
        });
        assert_eq!(warnings.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn 配了追踪导出时正常挂接_只有代码问题才告警() {
        let warnings = Arc::new(AtomicUsize::new(0));
        let provider = SdkTracerProvider::builder().build();
        let subscriber = tracing_subscriber::registry()
            .with(告警计数(warnings.clone()))
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));
        tracing::subscriber::with_default(subscriber, || {
            let fresh = tracing::info_span!("http.request");
            link_upstream(&fresh, opentelemetry::Context::new());
            assert_eq!(warnings.load(Ordering::SeqCst), 0);

            // span 已经开始后再挂上游是代码写错了，要告警。
            let started = tracing::info_span!("http.request");
            let _entered = started.enter();
            link_upstream(&started, opentelemetry::Context::new());
            assert_eq!(warnings.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn 接受有效上游关联标识() {
        let expected = Uuid::now_v7();
        let mut headers = HeaderMap::new();
        headers.insert(
            CORRELATION_ID_HEADER,
            HeaderValue::from_str(&expected.to_string()).expect("UUID 可写入头"),
        );

        assert_eq!(CorrelationId::from_headers(&headers).as_uuid(), expected);
    }

    #[test]
    fn 脏关联标识会替换为_uuidv7() {
        let mut headers = HeaderMap::new();
        headers.insert(CORRELATION_ID_HEADER, HeaderValue::from_static("invalid"));

        assert_eq!(
            CorrelationId::from_headers(&headers)
                .as_uuid()
                .get_version(),
            Some(Version::SortRand)
        );
    }
}
