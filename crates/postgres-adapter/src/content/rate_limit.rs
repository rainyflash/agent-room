use agent_room_application::ports::{
    ContentDownloadAttempt, ContentDownloadLimiter, ContentRateLimitDecision,
    ContentRateLimitFailure, ContentRateLimitFailureKind, ContentRateLimitResult, PortFuture,
};
use agent_room_domain::{
    content::MAX_CONTENT_BYTES,
    ids::PrincipalId,
    time::{DurationMillis, UtcMillis},
};
use sqlx::{PgPool, Postgres, Row, Transaction};
use thiserror::Error;

const OPERATION: &str = "content.download_limit";
const ADVISORY_LOCK_NAMESPACE: i64 = 0x0043_4F4E_5445_4E54;

/// 多副本共享的内容下载限流策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentDownloadLimitPolicy {
    window_millis: i64,
    max_downloads: u32,
    max_bytes: u64,
}

impl ContentDownloadLimitPolicy {
    /// 创建下载限流策略。
    ///
    /// # Errors
    ///
    /// 窗口无法存入 PostgreSQL、请求数超出数据库整数范围，或字节预算不足以容纳一个
    /// 合法最大对象时返回错误。
    pub fn new(
        window: DurationMillis,
        max_downloads: u32,
        max_bytes: u64,
    ) -> Result<Self, ContentDownloadLimitPolicyError> {
        let window_millis = i64::try_from(window.value())
            .map_err(|_| ContentDownloadLimitPolicyError::WindowOverflow)?;
        if max_downloads == 0 || i32::try_from(max_downloads).is_err() {
            return Err(ContentDownloadLimitPolicyError::InvalidDownloadCount);
        }
        if max_bytes < MAX_CONTENT_BYTES || i64::try_from(max_bytes).is_err() {
            return Err(ContentDownloadLimitPolicyError::InvalidByteBudget);
        }
        Ok(Self {
            window_millis,
            max_downloads,
            max_bytes,
        })
    }

    pub const fn window_millis(self) -> i64 {
        self.window_millis
    }

    pub const fn max_downloads(self) -> u32 {
        self.max_downloads
    }

    pub const fn max_bytes(self) -> u64 {
        self.max_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ContentDownloadLimitPolicyError {
    #[error("内容下载限流窗口超出 PostgreSQL 可表示范围")]
    WindowOverflow,
    #[error("内容下载次数预算必须处于 PostgreSQL 正整数范围")]
    InvalidDownloadCount,
    #[error("内容下载字节预算必须至少容纳一个最大合法对象")]
    InvalidByteBudget,
}

/// 使用 `PostgreSQL` 事务锁在全部控制平面副本之间共享下载预算。
#[derive(Clone)]
pub struct PostgresContentDownloadLimiter {
    pool: PgPool,
    policy: ContentDownloadLimitPolicy,
}

impl PostgresContentDownloadLimiter {
    pub const fn new(pool: PgPool, policy: ContentDownloadLimitPolicy) -> Self {
        Self { pool, policy }
    }

    async fn evaluate(
        &self,
        attempt: &ContentDownloadAttempt,
    ) -> ContentRateLimitResult<ContentRateLimitDecision> {
        let mut transaction = self.pool.begin().await.map_err(|_| unavailable())?;
        lock_principal(&mut transaction, attempt.principal_id).await?;
        let current = load_window(&mut transaction, attempt.principal_id).await?;
        let attempted_bytes = attempt.byte_length.value();

        let decision = match current {
            None => {
                let window =
                    DownloadWindow::start(attempt.attempted_at, attempted_bytes, self.policy)?;
                save_window(
                    &mut transaction,
                    attempt.principal_id,
                    &window,
                    attempt.attempted_at,
                )
                .await?;
                ContentRateLimitDecision::Allowed
            }
            Some(current) if attempt.attempted_at >= current.ends_at => {
                let window =
                    DownloadWindow::start(attempt.attempted_at, attempted_bytes, self.policy)?;
                save_window(
                    &mut transaction,
                    attempt.principal_id,
                    &window,
                    attempt.attempted_at,
                )
                .await?;
                ContentRateLimitDecision::Allowed
            }
            Some(current) if current.can_accept(attempted_bytes, self.policy) => {
                let updated = current.accept(attempted_bytes)?;
                save_window(
                    &mut transaction,
                    attempt.principal_id,
                    &updated,
                    attempt.attempted_at,
                )
                .await?;
                ContentRateLimitDecision::Allowed
            }
            Some(current) => ContentRateLimitDecision::RetryAt(current.ends_at),
        };

        transaction.commit().await.map_err(|_| unavailable())?;
        Ok(decision)
    }
}

impl ContentDownloadLimiter for PostgresContentDownloadLimiter {
    fn check<'a>(
        &'a self,
        attempt: &'a ContentDownloadAttempt,
    ) -> PortFuture<'a, ContentRateLimitResult<ContentRateLimitDecision>> {
        Box::pin(async move { self.evaluate(attempt).await })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DownloadWindow {
    started_at: UtcMillis,
    ends_at: UtcMillis,
    request_count: u32,
    byte_count: u64,
}

impl DownloadWindow {
    fn start(
        attempted_at: UtcMillis,
        attempted_bytes: u64,
        policy: ContentDownloadLimitPolicy,
    ) -> ContentRateLimitResult<Self> {
        let ends_at = attempted_at
            .value()
            .checked_add(policy.window_millis())
            .and_then(|value| UtcMillis::new(value).ok())
            .ok_or_else(unavailable)?;
        Ok(Self {
            started_at: attempted_at,
            ends_at,
            request_count: 1,
            byte_count: attempted_bytes,
        })
    }

    const fn can_accept(self, attempted_bytes: u64, policy: ContentDownloadLimitPolicy) -> bool {
        self.request_count < policy.max_downloads()
            && self.byte_count <= policy.max_bytes() - attempted_bytes
    }

    fn accept(self, attempted_bytes: u64) -> ContentRateLimitResult<Self> {
        Ok(Self {
            request_count: self.request_count.checked_add(1).ok_or_else(unavailable)?,
            byte_count: self
                .byte_count
                .checked_add(attempted_bytes)
                .ok_or_else(unavailable)?,
            ..self
        })
    }
}

async fn lock_principal(
    transaction: &mut Transaction<'_, Postgres>,
    principal_id: PrincipalId,
) -> ContentRateLimitResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, $2))")
        .bind(principal_id.to_string())
        .bind(ADVISORY_LOCK_NAMESPACE)
        .execute(&mut **transaction)
        .await
        .map_err(|_| unavailable())?;
    Ok(())
}

async fn load_window(
    transaction: &mut Transaction<'_, Postgres>,
    principal_id: PrincipalId,
) -> ContentRateLimitResult<Option<DownloadWindow>> {
    let row = sqlx::query(
        r"SELECT
              floor(extract(epoch FROM window_started_at) * 1000)::bigint AS started_at_ms,
              floor(extract(epoch FROM window_ends_at) * 1000)::bigint AS ends_at_ms,
              request_count,
              byte_count
          FROM agent_room.content_download_window
          WHERE principal_id = $1
          FOR UPDATE",
    )
    .bind(principal_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| unavailable())?;

    row.map(|row| {
        let started_at = decode_time(&row, "started_at_ms")?;
        let ends_at = decode_time(&row, "ends_at_ms")?;
        let request_count = row
            .try_get::<i32, _>("request_count")
            .ok()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(unavailable)?;
        let byte_count = row
            .try_get::<i64, _>("byte_count")
            .ok()
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(unavailable)?;
        Ok(DownloadWindow {
            started_at,
            ends_at,
            request_count,
            byte_count,
        })
    })
    .transpose()
}

async fn save_window(
    transaction: &mut Transaction<'_, Postgres>,
    principal_id: PrincipalId,
    window: &DownloadWindow,
    attempted_at: UtcMillis,
) -> ContentRateLimitResult<()> {
    let request_count = i32::try_from(window.request_count).map_err(|_| unavailable())?;
    let byte_count = i64::try_from(window.byte_count).map_err(|_| unavailable())?;
    sqlx::query(
        r"INSERT INTO agent_room.content_download_window (
              principal_id, window_started_at, window_ends_at,
              request_count, byte_count, last_attempt_at, version
          ) VALUES (
              $1,
              to_timestamp($2::double precision / 1000.0),
              to_timestamp($3::double precision / 1000.0),
              $4, $5,
              to_timestamp($6::double precision / 1000.0),
              0
          )
          ON CONFLICT (principal_id) DO UPDATE SET
              window_started_at = EXCLUDED.window_started_at,
              window_ends_at = EXCLUDED.window_ends_at,
              request_count = EXCLUDED.request_count,
              byte_count = EXCLUDED.byte_count,
              last_attempt_at = GREATEST(
                  agent_room.content_download_window.last_attempt_at,
                  EXCLUDED.last_attempt_at
              ),
              version = agent_room.content_download_window.version + 1",
    )
    .bind(principal_id.as_uuid())
    .bind(window.started_at.value())
    .bind(window.ends_at.value())
    .bind(request_count)
    .bind(byte_count)
    .bind(attempted_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|_| unavailable())?;
    Ok(())
}

fn decode_time(
    row: &sqlx::postgres::PgRow,
    column: &'static str,
) -> ContentRateLimitResult<UtcMillis> {
    row.try_get::<i64, _>(column)
        .ok()
        .and_then(|value| UtcMillis::new(value).ok())
        .ok_or_else(unavailable)
}

const fn unavailable() -> ContentRateLimitFailure {
    ContentRateLimitFailure::new(OPERATION, ContentRateLimitFailureKind::Unavailable)
}

#[cfg(test)]
mod tests {
    use agent_room_domain::{content::MAX_CONTENT_BYTES, time::DurationMillis};

    use super::{ContentDownloadLimitPolicy, ContentDownloadLimitPolicyError};

    #[test]
    fn 策略拒绝无法服务合法对象的预算() {
        let window = DurationMillis::new(60_000).expect("测试窗口有效");
        assert_eq!(
            ContentDownloadLimitPolicy::new(window, 0, MAX_CONTENT_BYTES),
            Err(ContentDownloadLimitPolicyError::InvalidDownloadCount)
        );
        assert_eq!(
            ContentDownloadLimitPolicy::new(window, 1, MAX_CONTENT_BYTES - 1),
            Err(ContentDownloadLimitPolicyError::InvalidByteBudget)
        );
    }

    #[test]
    fn 合法策略保留精确配置() {
        let window = DurationMillis::new(60_000).expect("测试窗口有效");
        let policy =
            ContentDownloadLimitPolicy::new(window, 30, 250 * 1024 * 1024).expect("标准策略有效");
        assert_eq!(policy.window_millis(), 60_000);
        assert_eq!(policy.max_downloads(), 30);
        assert_eq!(policy.max_bytes(), 250 * 1024 * 1024);
    }
}
