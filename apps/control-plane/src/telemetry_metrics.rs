use std::time::Instant;

use axum::{
    body::Body,
    extract::{MatchedPath, Request, State},
    http::{Method, StatusCode},
    middleware::Next,
    response::Response,
};
use opentelemetry::{
    KeyValue,
    metrics::{Counter, Gauge, Histogram},
};

use crate::SERVICE_NAME;

#[cfg(test)]
const ALLOWED_LABEL_KEYS: [&str; 6] = [
    "dependency",
    "http.request.method",
    "http.response.status_code",
    "http.route",
    "metric",
    "surface",
];

#[derive(Clone)]
pub(crate) struct TelemetryMetrics {
    api_requests: Counter<u64>,
    api_duration: Histogram<f64>,
    dependency_available: Gauge<u64>,
    dependency_duration: Histogram<f64>,
    operational_value: Gauge<i64>,
    operational_age: Gauge<f64>,
    sampler_failures: Counter<u64>,
    frontend_duration: Histogram<f64>,
    frontend_score: Histogram<f64>,
}

impl TelemetryMetrics {
    pub(crate) fn new() -> Self {
        let meter = opentelemetry::global::meter(SERVICE_NAME);
        Self {
            api_requests: meter
                .u64_counter("agent_room.api.server.requests")
                .with_description("按固定路由、方法和状态码统计的控制平面请求数")
                .build(),
            api_duration: meter
                .f64_histogram("agent_room.api.server.duration")
                .with_description("控制平面请求耗时")
                .with_unit("s")
                .with_boundaries(vec![
                    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 0.8, 1.0, 2.5, 5.0,
                ])
                .build(),
            dependency_available: meter
                .u64_gauge("agent_room.dependency.available")
                .with_description("依赖最近一次探测是否可用")
                .build(),
            dependency_duration: meter
                .f64_histogram("agent_room.dependency.probe.duration")
                .with_description("依赖探测耗时")
                .with_unit("s")
                .with_boundaries(vec![
                    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
                ])
                .build(),
            operational_value: meter
                .i64_gauge("agent_room.operational.value")
                .with_description("数据库、Outbox、投影和内容生命周期的低基数运行值")
                .build(),
            operational_age: meter
                .f64_gauge("agent_room.operational.age")
                .with_description("队列或投影最旧项目的年龄")
                .with_unit("s")
                .build(),
            sampler_failures: meter
                .u64_counter("agent_room.operational.sampler.failures")
                .with_description("运行指标采样失败次数")
                .build(),
            frontend_duration: meter
                .f64_histogram("agent_room.frontend.duration")
                .with_description("前端交互和渲染耗时")
                .with_unit("s")
                .with_boundaries(vec![0.05, 0.1, 0.25, 0.5, 0.8, 1.0, 2.5, 4.0, 8.0, 15.0])
                .build(),
            frontend_score: meter
                .f64_histogram("agent_room.frontend.score")
                .with_description("无身份维度的前端布局偏移分数")
                .with_boundaries(vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0])
                .build(),
        }
    }

    pub(crate) fn record_dependency(
        &self,
        dependency: &'static str,
        available: bool,
        latency_millis: u64,
    ) {
        let attributes = [KeyValue::new("dependency", dependency)];
        self.dependency_available
            .record(u64::from(available), &attributes);
        self.dependency_duration.record(
            std::time::Duration::from_millis(latency_millis).as_secs_f64(),
            &attributes,
        );
    }

    pub(crate) fn record_operational_value(&self, metric: &'static str, value: i64) {
        self.operational_value
            .record(value, &[KeyValue::new("metric", metric)]);
    }

    pub(crate) fn record_operational_age(&self, metric: &'static str, seconds: f64) {
        self.operational_age
            .record(seconds.max(0.0), &[KeyValue::new("metric", metric)]);
    }

    pub(crate) fn record_sampler_failure(&self) {
        self.sampler_failures.add(1, &[]);
    }

    pub(crate) fn record_frontend(
        &self,
        metric: &'static str,
        surface: &'static str,
        value: f64,
        is_score: bool,
    ) {
        let attributes = [
            KeyValue::new("metric", metric),
            KeyValue::new("surface", surface),
        ];
        if is_score {
            self.frontend_score.record(value, &attributes);
        } else {
            self.frontend_duration.record(value / 1_000.0, &attributes);
        }
    }

    fn record_http(&self, method: &Method, route: &str, status: StatusCode, elapsed: f64) {
        let attributes = [
            KeyValue::new("http.request.method", method_label(method)),
            KeyValue::new("http.route", route.to_owned()),
            KeyValue::new("http.response.status_code", i64::from(status.as_u16())),
        ];
        self.api_requests.add(1, &attributes);
        self.api_duration.record(elapsed, &attributes);
    }
}

pub(crate) async fn record_http_request(
    State(metrics): State<TelemetryMetrics>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let started = Instant::now();
    let method = request.method().clone();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("<unmatched>", MatchedPath::as_str)
        .to_owned();
    let response = next.run(request).await;
    metrics.record_http(
        &method,
        &route,
        response.status(),
        started.elapsed().as_secs_f64(),
    );
    response
}

fn method_label(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::DELETE => "DELETE",
        Method::PATCH => "PATCH",
        Method::HEAD => "HEAD",
        Method::OPTIONS => "OPTIONS",
        _ => "OTHER",
    }
}

#[cfg(test)]
mod tests {
    use super::{ALLOWED_LABEL_KEYS, method_label};
    use axum::http::Method;

    #[test]
    fn 指标标签契约不允许身份正文令牌或本地路径() {
        for forbidden in [
            "user",
            "principal",
            "agent",
            "room",
            "event",
            "message",
            "token",
            "path",
            "url",
            "filename",
        ] {
            assert!(
                ALLOWED_LABEL_KEYS
                    .iter()
                    .all(|label| !label.contains(forbidden)),
                "禁止的标签语义：{forbidden}"
            );
        }
    }

    #[test]
    fn 任意扩展方法收敛为固定标签() {
        let extension = Method::from_bytes(b"CUSTOM-USER-123").expect("测试方法有效");
        assert_eq!(method_label(&extension), "OTHER");
    }
}
