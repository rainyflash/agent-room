#!/bin/sh
set -eu
umask 077

fail() {
  echo "数据库备份失败：$1" >&2
  exit 1
}

read_secret() {
  path="$1"
  [ -s "$path" ] || fail "缺少 Secret $path"
  value=$(cat "$path")
  [ -n "$value" ] || fail "Secret 为空 $path"
  printf '%s' "$value"
}

case "${AGENT_ROOM_BACKUP_ID:-}" in
  [0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]T[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]Z-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]) ;;
  *) fail "备份 ID 格式无效" ;;
esac

target="/backup/.partial-${AGENT_ROOM_BACKUP_ID}/database"
mkdir -p "$target"

dump_database() {
  database="$1"
  username="$2"
  secret_path="$3"
  output="$4"
  PGPASSWORD=$(read_secret "$secret_path") \
    pg_dump \
      --host "$AGENT_ROOM_DB_HOST" \
      --port "$AGENT_ROOM_DB_PORT" \
      --username "$username" \
      --dbname "$database" \
      --format custom \
      --compress zstd:6 \
      --no-password \
      --file "$target/$output"
}

export PGSSLMODE="$AGENT_ROOM_DB_TLS_MODE"
dump_database "$AGENT_ROOM_DB_NAME" "$AGENT_ROOM_DB_MIGRATION_USER" \
  /run/secrets/agent_room_db_migration_password agent-room.dump
PGPASSWORD=$(read_secret /run/secrets/synapse_db_password) \
  pg_dump \
    --host "$AGENT_ROOM_DB_HOST" \
    --port "$AGENT_ROOM_DB_PORT" \
    --username "$SYNAPSE_DB_USER" \
    --dbname "$SYNAPSE_DB_NAME" \
    --format custom \
    --compress zstd:6 \
    --exclude-table-data public.e2e_one_time_keys_json \
    --no-password \
    --file "$target/synapse.dump"
dump_database "$KEYCLOAK_DB_NAME" "$KEYCLOAK_DB_USER" \
  /run/secrets/keycloak_db_password keycloak.dump

cat >"$target/README.txt" <<'EOF'
三个自定义格式归档分别是 Agent Room、Synapse 和 Keycloak 的权威逻辑快照。
Synapse 的一次性 E2EE 密钥表按官方恢复建议排除；Keycloak CLI 在线导出不是权威备份。
EOF
