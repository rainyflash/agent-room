use std::{collections::BTreeSet, sync::Arc, time::Instant};

use futures_util::future::join_all;
use thiserror::Error;

use crate::ports::PortFuture;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DependencyKind {
    PostgreSql,
    Matrix,
    ObjectStore,
}

impl DependencyKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PostgreSql => "postgresql",
            Self::Matrix => "matrix",
            Self::ObjectStore => "object_store",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeFailureKind {
    Timeout,
    Connection,
    RejectedResponse,
    InvalidResponse,
    Internal,
}

impl ProbeFailureKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Connection => "connection",
            Self::RejectedResponse => "rejected_response",
            Self::InvalidResponse => "invalid_response",
            Self::Internal => "internal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeFailure {
    kind: ProbeFailureKind,
}

impl ProbeFailure {
    pub const fn new(kind: ProbeFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> ProbeFailureKind {
        self.kind
    }
}

pub type ProbeResult = Result<(), ProbeFailure>;

pub trait DependencyProbe: Send + Sync {
    fn dependency(&self) -> DependencyKind;

    fn check<'a>(&'a self, correlation_id: &'a str) -> PortFuture<'a, ProbeResult>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyState {
    Ready,
    Unavailable,
}

impl DependencyState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DependencyCheck {
    dependency: DependencyKind,
    state: DependencyState,
    latency_millis: u64,
    failure: Option<ProbeFailureKind>,
}

impl DependencyCheck {
    pub const fn dependency(self) -> DependencyKind {
        self.dependency
    }

    pub const fn state(self) -> DependencyState {
        self.state
    }

    pub const fn latency_millis(self) -> u64 {
        self.latency_millis
    }

    pub const fn failure(self) -> Option<ProbeFailureKind> {
        self.failure
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessState {
    Ready,
    Degraded,
}

impl ReadinessState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Degraded => "degraded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessReport {
    state: ReadinessState,
    checks: Vec<DependencyCheck>,
}

impl ReadinessReport {
    pub const fn state(&self) -> ReadinessState {
        self.state
    }

    pub fn checks(&self) -> &[DependencyCheck] {
        &self.checks
    }

    pub const fn is_ready(&self) -> bool {
        matches!(self.state, ReadinessState::Ready)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum HealthConfigurationError {
    #[error("健康模型至少需要一个依赖探针")]
    Empty,
    #[error("依赖探针重复：{dependency}")]
    Duplicate { dependency: &'static str },
}

pub struct ReadinessService {
    probes: Vec<Arc<dyn DependencyProbe>>,
}

impl ReadinessService {
    /// 创建依赖就绪聚合服务。
    ///
    /// # Errors
    ///
    /// 探针为空或同一依赖被重复注册时返回配置错误。
    pub fn new(probes: Vec<Arc<dyn DependencyProbe>>) -> Result<Self, HealthConfigurationError> {
        if probes.is_empty() {
            return Err(HealthConfigurationError::Empty);
        }

        let mut dependencies = BTreeSet::new();
        for probe in &probes {
            let dependency = probe.dependency();
            if !dependencies.insert(dependency) {
                return Err(HealthConfigurationError::Duplicate {
                    dependency: dependency.as_str(),
                });
            }
        }

        Ok(Self { probes })
    }

    pub async fn check(&self, correlation_id: &str) -> ReadinessReport {
        let checks = join_all(self.probes.iter().map(|probe| async move {
            let started_at = Instant::now();
            let result = probe.check(correlation_id).await;
            let latency_millis =
                u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX);

            match result {
                Ok(()) => DependencyCheck {
                    dependency: probe.dependency(),
                    state: DependencyState::Ready,
                    latency_millis,
                    failure: None,
                },
                Err(failure) => DependencyCheck {
                    dependency: probe.dependency(),
                    state: DependencyState::Unavailable,
                    latency_millis,
                    failure: Some(failure.kind()),
                },
            }
        }))
        .await;

        let mut checks = checks;
        checks.sort_by_key(|check| check.dependency());
        let state = if checks
            .iter()
            .all(|check| check.state() == DependencyState::Ready)
        {
            ReadinessState::Ready
        } else {
            ReadinessState::Degraded
        };

        ReadinessReport { state, checks }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        DependencyKind, DependencyProbe, DependencyState, HealthConfigurationError, ProbeFailure,
        ProbeFailureKind, ProbeResult, ReadinessService, ReadinessState,
    };
    use crate::ports::PortFuture;

    struct StaticProbe {
        dependency: DependencyKind,
        result: ProbeResult,
    }

    impl DependencyProbe for StaticProbe {
        fn dependency(&self) -> DependencyKind {
            self.dependency
        }

        fn check<'a>(&'a self, _correlation_id: &'a str) -> PortFuture<'a, ProbeResult> {
            Box::pin(async move { self.result })
        }
    }

    fn probe(dependency: DependencyKind, result: ProbeResult) -> Arc<dyn DependencyProbe> {
        Arc::new(StaticProbe { dependency, result })
    }

    #[tokio::test]
    async fn 任一依赖失败时报告精确降级层() {
        let service = ReadinessService::new(vec![
            probe(DependencyKind::ObjectStore, Ok(())),
            probe(
                DependencyKind::PostgreSql,
                Err(ProbeFailure::new(ProbeFailureKind::Connection)),
            ),
            probe(DependencyKind::Matrix, Ok(())),
        ])
        .expect("探针配置有效");

        let report = service.check("01945c1e-7b5a-7c7f-8a28-2de53f56a9ad").await;

        assert_eq!(report.state(), ReadinessState::Degraded);
        assert_eq!(report.checks().len(), 3);
        let postgresql = report
            .checks()
            .iter()
            .find(|check| check.dependency() == DependencyKind::PostgreSql)
            .expect("必须返回 PostgreSQL 层");
        assert_eq!(postgresql.state(), DependencyState::Unavailable);
        assert_eq!(postgresql.failure(), Some(ProbeFailureKind::Connection));
    }

    #[test]
    fn 重复探针在启动前失败() {
        let result = ReadinessService::new(vec![
            probe(DependencyKind::Matrix, Ok(())),
            probe(DependencyKind::Matrix, Ok(())),
        ]);

        assert!(matches!(
            result,
            Err(HealthConfigurationError::Duplicate {
                dependency: "matrix"
            })
        ));
    }
}
