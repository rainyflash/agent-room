//! 每个账户的邮件提醒开关（`specs/redesign/reminders.md` 的“存什么”）。没有这一行的照默认算：开着。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{PortFuture, ReminderAccount, ReminderSettingsRepository},
};
use agent_room_domain::{ids::PrincipalId, time::UtcMillis};

use crate::{PostgresRepositories, agents::corrupt_data, error::map_sqlx_error};

/// 还在用的账户：被停用的也算，正在删、已经删了的不算。
const LIVE_STATUSES: [&str; 2] = ["active", "suspended"];

impl ReminderSettingsRepository for PostgresRepositories {
    fn find_reminder_account(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<ReminderAccount>> {
        Box::pin(async move {
            let operation = "reminders.find_account";
            let row = sqlx::query_as::<_, (String, Option<bool>)>(
                r"SELECT principal.status, reminder.email_enabled
                    FROM agent_room.principal AS principal
                    LEFT JOIN agent_room.account_reminder AS reminder
                      ON reminder.principal_id = principal.id
                   WHERE principal.id = $1",
            )
            .bind(principal_id.as_uuid())
            .fetch_optional(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            match row {
                None => Ok(ReminderAccount::Gone),
                Some((status, enabled)) if LIVE_STATUSES.contains(&status.as_str()) => {
                    Ok(ReminderAccount::Active {
                        email_enabled: enabled.unwrap_or(true),
                    })
                }
                Some((status, _)) if matches!(status.as_str(), "deleting" | "deleted") => {
                    Ok(ReminderAccount::Gone)
                }
                Some(_) => Err(corrupt_data(operation)),
            }
        })
    }

    fn set_email_reminders(
        &self,
        principal_id: PrincipalId,
        enabled: bool,
        changed_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        Box::pin(async move {
            let operation = "reminders.set_email";
            // 账户还在用才写；和现在一样就不动，免得改的时间被重复的点击刷新。
            let live = sqlx::query_scalar::<_, bool>(
                r"WITH account AS (
                      SELECT id FROM agent_room.principal
                       WHERE id = $1 AND status = ANY($4::text[])
                  ), written AS (
                      INSERT INTO agent_room.account_reminder
                             (principal_id, email_enabled, email_changed_at, updated_at)
                      SELECT id, $2, to_timestamp($3::double precision / 1000.0),
                             to_timestamp($3::double precision / 1000.0)
                        FROM account
                      ON CONFLICT (principal_id) DO UPDATE
                         SET email_enabled = EXCLUDED.email_enabled,
                             email_changed_at = EXCLUDED.email_changed_at,
                             updated_at = EXCLUDED.updated_at
                       WHERE agent_room.account_reminder.email_enabled
                             IS DISTINCT FROM EXCLUDED.email_enabled
                      RETURNING 1
                  )
                  SELECT EXISTS (SELECT 1 FROM account)",
            )
            .bind(principal_id.as_uuid())
            .bind(enabled)
            .bind(changed_at.value())
            .bind(LIVE_STATUSES.as_slice())
            .fetch_one(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            Ok(live)
        })
    }
}
