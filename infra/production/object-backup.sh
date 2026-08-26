#!/bin/sh
set -eu
umask 077

fail() {
  echo "对象备份失败：$1" >&2
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

target="/backup/.partial-${AGENT_ROOM_BACKUP_ID}/objects"
mkdir -p "$target/data" /tmp/mc
access_key=$(read_secret /run/secrets/s3_access_key)
secret_key=$(read_secret /run/secrets/s3_secret_key)

mc --config-dir /tmp/mc alias set source "$AGENT_ROOM_CONTENT_S3_ENDPOINT" \
  "$access_key" "$secret_key" --api S3v4 >/dev/null
mc --config-dir /tmp/mc ls --recursive --json \
  "source/$AGENT_ROOM_CONTENT_S3_BUCKET" >"$target/source-inventory.ndjson"
mc --config-dir /tmp/mc mirror --overwrite --preserve \
  "source/$AGENT_ROOM_CONTENT_S3_BUCKET" "$target/data"
