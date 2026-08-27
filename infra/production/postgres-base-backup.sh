#!/bin/sh
set -eu
umask 077

fail() {
  echo "PostgreSQL 物理备份失败：$1" >&2
  exit 1
}

read_secret() {
  [ -s "$1" ] || fail "缺少 Secret $1"
  value=$(cat "$1")
  [ -n "$value" ] || fail "Secret 为空 $1"
  printf '%s' "$value"
}

case "${AGENT_ROOM_BACKUP_ID:-}" in
  [0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]T[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]Z-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]) ;;
  *) fail "备份 ID 格式无效" ;;
esac

target="/backup/.partial-${AGENT_ROOM_BACKUP_ID}/postgres"
base="$target/base"
mkdir -p "$target"
[ ! -e "$base" ] || fail "物理备份目标已存在"

export PGSSLMODE="$AGENT_ROOM_DB_TLS_MODE"
export PGPASSWORD
PGPASSWORD=$(read_secret /run/secrets/postgres_bootstrap_password)
pg_basebackup \
  --host "$AGENT_ROOM_DB_HOST" \
  --port "$AGENT_ROOM_DB_PORT" \
  --username agent_room_bootstrap \
  --pgdata "$base" \
  --format plain \
  --wal-method stream \
  --checkpoint fast \
  --manifest-checksums SHA256 \
  --no-password
pg_verifybackup --exit-on-error "$base"

start_wal=$(sed -n 's/^START WAL LOCATION: .* (file \([0-9A-F]\{24\}\))$/\1/p' "$base/backup_label")
printf '%s' "$start_wal" | grep -Eq '^[0-9A-F]{24}$' || fail "基础备份起始 WAL 无效"

restore_name=$(printf 'agent_room_%s' "$AGENT_ROOM_BACKUP_ID" | tr 'TZ-' '___')
restore_lsn=$(psql \
  --host "$AGENT_ROOM_DB_HOST" \
  --port "$AGENT_ROOM_DB_PORT" \
  --username agent_room_bootstrap \
  --dbname postgres \
  --no-password \
  --tuples-only \
  --no-align \
  --command "SELECT pg_create_restore_point('$restore_name')")
wal_file=$(psql \
  --host "$AGENT_ROOM_DB_HOST" \
  --port "$AGENT_ROOM_DB_PORT" \
  --username agent_room_bootstrap \
  --dbname postgres \
  --no-password \
  --tuples-only \
  --no-align \
  --command "SELECT pg_walfile_name(pg_switch_wal())")
printf '%s' "$wal_file" | grep -Eq '^[0-9A-F]{24}$' || fail "恢复点末端 WAL 无效"
[ "$start_wal" \> "$wal_file" ] && fail "WAL 恢复区间倒置"

archived=false
attempt=0
while [ "$attempt" -lt 180 ]; do
  if [ -s "/archive/$wal_file" ]; then
    archived=true
    break
  fi
  attempt=$((attempt + 1))
  sleep 1
done
[ "$archived" = true ] || fail "恢复点 WAL 未在 180 秒内归档"

mkdir -p "$target/wal"
copied_wal=0
for source in /archive/[0-9A-F]*; do
  [ -f "$source" ] || continue
  name=${source##*/}
  printf '%s' "$name" | grep -Eq '^[0-9A-F]{24}$' || continue
  [ "$name" \< "$start_wal" ] && continue
  [ "$name" \> "$wal_file" ] && continue
  cp -a "$source" "$target/wal/$name"
  copied_wal=$((copied_wal + 1))
done
[ "$copied_wal" -gt 0 ] || fail "没有复制任何恢复所需 WAL"
[ -s "$target/wal/$start_wal" ] || fail "缺少基础备份起始 WAL"
[ -s "$target/wal/$wal_file" ] || fail "缺少恢复点末端 WAL"
cat >"$target/restore-point.json" <<EOF
{
  "name": "$restore_name",
  "lsn": "$restore_lsn",
  "lastRequiredWal": "$wal_file"
}
EOF
