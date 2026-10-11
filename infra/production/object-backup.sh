#!/bin/sh
set -eu
umask 077

# 对象只传新的（specs/backups/design.md）：rclone sync 到仓库里的一份镜像，被删掉或者被覆盖的旧版本
# 挪进当天的目录，过了保留期由备份清理删掉。每套快照只记一份清单：同步完以后镜像里有哪些对象、
# 各自的 SHA-256。恢复时照清单从镜像和快照之后才挪走的旧版本里取回。

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

# 全量记下同步完镜像的清单；只打恢复点的时候紧跟着恢复点同步一次，不记清单。
kind="${AGENT_ROOM_BACKUP_KIND:-full}"
case "$kind" in
  full | point) ;;
  *) fail "备份种类无效" ;;
esac

target="/backup/.partial-${AGENT_ROOM_BACKUP_ID}/objects"
mirror=/backup/objects/mirror
removed="/backup/objects/removed/$(date -u +%Y-%m-%d)"
mkdir -p "$mirror" /tmp/rclone

# rclone 只从环境变量读配置：不写配置文件，密钥不出现在命令行里。根目录只读，缓存放 /tmp。
export HOME=/tmp/rclone XDG_CACHE_HOME=/tmp/rclone/cache RCLONE_CONFIG=/tmp/rclone/rclone.conf
export RCLONE_CONFIG_SOURCE_TYPE=s3 RCLONE_CONFIG_SOURCE_PROVIDER=SeaweedFS
export RCLONE_CONFIG_SOURCE_ENDPOINT="$AGENT_ROOM_CONTENT_S3_ENDPOINT"
RCLONE_CONFIG_SOURCE_ACCESS_KEY_ID=$(read_secret /run/secrets/s3_access_key)
RCLONE_CONFIG_SOURCE_SECRET_ACCESS_KEY=$(read_secret /run/secrets/s3_secret_key)
export RCLONE_CONFIG_SOURCE_ACCESS_KEY_ID RCLONE_CONFIG_SOURCE_SECRET_ACCESS_KEY

# 同步中途出错时 rclone 不删镜像里的任何东西；要删、要覆盖的先挪进当天的目录。内容对象的键每次上传
# 都不一样，不会被覆盖；万一同一天同一个键换了两次，前一版就没了，恢复时按 SHA-256 核对会报出来。
# 一次列完整个桶（--fast-list），比较时用列表里的修改时间（--use-server-modtime），没变的对象
# 不用再一个个去问。
rclone sync "source:$AGENT_ROOM_CONTENT_S3_BUCKET" "$mirror" --backup-dir "$removed" \
  --fast-list --use-server-modtime
[ "$kind" = full ] || exit 0

mkdir -p "$target"
# 清单每行一个对象（rclone lsjson 每项一行，去掉首尾的方括号和行尾逗号）。列的是同步完的镜像，
# 不是对象存储：列和同步之间被删掉的对象不会出现在清单里却取不回来。
rclone lsjson --recursive --files-only --no-mimetype --hash --hash-type sha256 "$mirror" >"$target/.inventory.json"
sed -e '/^\[$/d' -e '/^\]$/d' -e 's/,$//' "$target/.inventory.json" >"$target/source-inventory.ndjson"
rm "$target/.inventory.json"
