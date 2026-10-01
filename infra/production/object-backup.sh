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
mkdir -p "$target/data" /tmp/rclone

# rclone 只从环境变量读配置：不写配置文件，密钥不出现在命令行里。根目录只读，缓存放 /tmp。
export HOME=/tmp/rclone XDG_CACHE_HOME=/tmp/rclone/cache RCLONE_CONFIG=/tmp/rclone/rclone.conf
export RCLONE_CONFIG_SOURCE_TYPE=s3 RCLONE_CONFIG_SOURCE_PROVIDER=SeaweedFS
export RCLONE_CONFIG_SOURCE_ENDPOINT="$AGENT_ROOM_CONTENT_S3_ENDPOINT"
RCLONE_CONFIG_SOURCE_ACCESS_KEY_ID=$(read_secret /run/secrets/s3_access_key)
RCLONE_CONFIG_SOURCE_SECRET_ACCESS_KEY=$(read_secret /run/secrets/s3_secret_key)
export RCLONE_CONFIG_SOURCE_ACCESS_KEY_ID RCLONE_CONFIG_SOURCE_SECRET_ACCESS_KEY

source_bucket="source:$AGENT_ROOM_CONTENT_S3_BUCKET"
# 清单每行一个对象（rclone lsjson 每项一行，去掉首尾的方括号和行尾逗号）。
rclone lsjson --recursive --files-only --no-mimetype "$source_bucket" >/tmp/rclone/inventory.json
sed -e '/^\[$/d' -e '/^\]$/d' -e 's/,$//' /tmp/rclone/inventory.json >"$target/source-inventory.ndjson"
rclone copy --no-check-dest "$source_bucket" "$target/data"
