#!/bin/sh
set -eu
umask 077

socket_directory="${AGENT_ROOM_RESTORE_SOCKET_DIRECTORY:-/tmp}"
snapshot=/tmp/agent-room-membership-projection.csv

# Synapse 是房间成员状态的权威事实源；恢复后不能把旧投影当成事实继续使用。
psql \
  --host "$socket_directory" \
  --username agent_room_bootstrap \
  --dbname synapse \
  --set ON_ERROR_STOP=1 <<SQL
COPY (
  SELECT member.room_id,
         member.state_key AS matrix_user_id,
         member.event_id,
         member.membership,
         greatest(
           -100,
           least(
             100,
             coalesce(
               (power_json.json::jsonb -> 'content' -> 'users' ->> member.state_key)::integer,
               (power_json.json::jsonb -> 'content' ->> 'users_default')::integer,
               0
             )
           )
         ) AS power_level
    FROM current_state_events AS member
    LEFT JOIN current_state_events AS power
      ON power.room_id = member.room_id
     AND power.type = 'm.room.power_levels'
     AND power.state_key = ''
    LEFT JOIN event_json AS power_json ON power_json.event_id = power.event_id
   WHERE member.type = 'm.room.member'
     AND member.membership IN ('invite', 'join', 'leave', 'ban', 'knock')
) TO '$snapshot' WITH (FORMAT csv, HEADER true);
SQL

psql \
  --host "$socket_directory" \
  --username agent_room_bootstrap \
  --dbname agent_room \
  --tuples-only \
  --no-align \
  --set ON_ERROR_STOP=1 <<SQL
BEGIN;
CREATE TEMP TABLE restored_matrix_membership (
  matrix_room_id text NOT NULL,
  matrix_user_id text NOT NULL,
  event_id text NOT NULL,
  membership text NOT NULL,
  power_level integer NOT NULL
) ON COMMIT DROP;
COPY restored_matrix_membership FROM '$snapshot' WITH (FORMAT csv, HEADER true);

LOCK TABLE agent_room.room_membership_projection IN EXCLUSIVE MODE;
DELETE FROM agent_room.matrix_projection_event_receipt
 WHERE consumer_name = 'matrix-room-projection-v1';
DELETE FROM agent_room.room_membership_projection;
DELETE FROM agent_room.matrix_projection_cursor
 WHERE consumer_name = 'matrix-room-projection-v1';

INSERT INTO agent_room.room_membership_projection (
  room_instance_id,
  agent_id,
  matrix_membership,
  power_level,
  last_event_id,
  projected_at
)
SELECT room.id,
       agent.id,
       restored.membership,
       restored.power_level,
       restored.event_id,
       clock_timestamp()
  FROM restored_matrix_membership AS restored
  JOIN agent_room.room_instance AS room
    ON room.matrix_room_id = restored.matrix_room_id
  JOIN agent_room.agent AS agent
    ON agent.matrix_user_id = restored.matrix_user_id;

UPDATE agent_room.room_instance AS room
   SET member_count_projection = (
         SELECT count(*)::integer
           FROM agent_room.room_membership_projection AS membership
          WHERE membership.room_instance_id = room.id
            AND membership.matrix_membership = 'join'
       ),
       activity_score = 0,
       updated_at = greatest(room.updated_at, clock_timestamp()),
       version = room.version + 1;
COMMIT;

SELECT 'memberships=' || count(*)
  FROM agent_room.room_membership_projection;
SELECT 'rooms=' || count(*)
  FROM agent_room.room_instance;
SQL

# 不伪造 Matrix /sync token。缺失游标会让运行时走权威 Matrix 回源并执行完整同步。
rm -f "$snapshot"
