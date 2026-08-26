#!/bin/sh
set -eu
umask 077

socket_directory="${AGENT_ROOM_RESTORE_SOCKET_DIRECTORY:-/tmp}"

# 旧恢复点可能早于账户删除完成时间。恢复环境必须先撤销这些主体，再允许服务对外开放。
psql \
  --host "$socket_directory" \
  --username agent_room_bootstrap \
  --dbname agent_room \
  --tuples-only \
  --no-align \
  --set ON_ERROR_STOP=1 <<'SQL'
BEGIN;
CREATE TEMP TABLE restored_account_deletion_ledger (
  job_id uuid PRIMARY KEY,
  principal_id uuid UNIQUE NOT NULL,
  matrix_user_id text NOT NULL,
  completed_at timestamptz NOT NULL
) ON COMMIT DROP;

INSERT INTO restored_account_deletion_ledger
SELECT (entry ->> 'jobId')::uuid,
       (entry ->> 'principalId')::uuid,
       entry ->> 'matrixUserId',
       (entry ->> 'completedAt')::timestamptz
  FROM jsonb_array_elements(
         pg_read_file('/wal/account-deletions.json')::jsonb -> 'entries'
       ) AS entry;

CREATE TEMP TABLE deletion_replay_candidate ON COMMIT DROP AS
SELECT ledger.*
  FROM restored_account_deletion_ledger AS ledger
  JOIN agent_room.principal AS principal ON principal.id = ledger.principal_id
 WHERE principal.status <> 'deleted'
   AND NOT EXISTS (
         SELECT 1
           FROM agent_room.account_deletion_job AS job
          WHERE job.principal_id = ledger.principal_id
       );

UPDATE agent_room.principal AS principal
   SET status = 'deleting',
       updated_at = greatest(principal.updated_at, clock_timestamp()),
       version = principal.version + 1
 WHERE principal.id IN (
       SELECT ledger.principal_id FROM restored_account_deletion_ledger AS ledger
     )
   AND principal.status IN ('active', 'suspended');

INSERT INTO agent_room.account_deletion_job (
  id,
  principal_id,
  matrix_user_id,
  receipt_digest,
  requested_at,
  updated_at
)
SELECT candidate.job_id,
       candidate.principal_id,
       candidate.matrix_user_id,
       decode(
         md5(candidate.job_id::text || ':restore:1') ||
         md5(candidate.job_id::text || ':restore:2'),
         'hex'
       ),
       least(candidate.completed_at, clock_timestamp()),
       clock_timestamp()
  FROM deletion_replay_candidate AS candidate;

SELECT 'deletion_ledger_entries=' || count(*)
  FROM restored_account_deletion_ledger;
SELECT 'deletion_replays_queued=' || count(*)
  FROM deletion_replay_candidate;
COMMIT;
SQL
