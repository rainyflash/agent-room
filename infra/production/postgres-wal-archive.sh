#!/bin/sh
set -eu
umask 077

# 一直留着的 WAL（specs/backups/design.md）。每次定时备份都打一个恢复点、切一次 WAL，再把
# 从上一个恢复点到这个恢复点之间的段用 pg_waldump 接着读一遍。读通了才压缩、解压核对、记下
# SHA-256 放进 wal-store/，往恢复点记录里追加一行，最后才删归档目录里的原文件。读不通就停下
# 报错，什么都不删：PostgreSQL 会一直留着没归档完的段，归档目录里的也都还在，修好以后接着来。

fail() {
  echo "WAL 归档失败：$1" >&2
  exit 1
}

read_secret() {
  [ -s "$1" ] || fail "缺少 Secret $1"
  value=$(cat "$1")
  [ -n "$value" ] || fail "Secret 为空 $1"
  printf '%s' "$value"
}

matches() {
  printf '%s' "$1" | grep -Eq "$2"
}

backup_id() {
  case "$1" in
    [0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]T[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]Z-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]) return 0 ;;
    *) return 1 ;;
  esac
}

backup_id "${AGENT_ROOM_BACKUP_ID:-}" || fail "恢复点 ID 格式无效"

SEGMENT='^[0-9A-F]{24}$'
LSN='^[0-9A-F]{1,8}/[0-9A-F]{1,8}$'
archive=/backup/wal
store=/backup/wal-store
check=/backup/.wal-check
log="$store/restore-points.log"
owner="${AGENT_ROOM_HOST_UID:-0}:${AGENT_ROOM_HOST_GID:-0}"

[ -d "$archive" ] || fail "缺少 WAL 归档目录"
mkdir -p "$store"
rm -rf "$check"
rm -f "$store"/.*.partial
# 核对用的目录和归档目录在同一个挂载里，归档的段硬链接过来，不多占地方。
mkdir "$check"

export PGSSLMODE="$AGENT_ROOM_DB_TLS_MODE"
export PGPASSWORD
PGPASSWORD=$(read_secret /run/secrets/postgres_bootstrap_password)

query() {
  psql \
    --host "$AGENT_ROOM_DB_HOST" \
    --port "$AGENT_ROOM_DB_PORT" \
    --username agent_room_bootstrap \
    --dbname postgres \
    --no-password \
    --tuples-only \
    --no-align \
    --set ON_ERROR_STOP=1 \
    --command "$1"
}

# 接着哪里读：恢复点记录的最后一行。还没有记录时，从这次备份刚做好的那套全量的恢复点接起：
# 全量是从归档目录复制的 WAL，那几段还在归档目录里。
anchor=""
if [ -s "$log" ]; then
  # 每行：时间 种类 恢复点名字 LSN 到哪个段为止 全量 ID（没有就是 -）
  set -- $(tail -n 1 "$log")
  [ "$#" -eq 6 ] || fail "恢复点记录的最后一行格式不对"
  from_lsn=$4
  last_segment=$5
else
  [ -n "${AGENT_ROOM_WAL_ANCHOR:-}" ] || fail "还没有恢复点记录，也没有给出从哪套全量接起"
  set -- $AGENT_ROOM_WAL_ANCHOR
  [ "$#" -eq 4 ] || fail "接起的全量格式不对"
  anchor_backup=$1
  anchor_name=$2
  from_lsn=$3
  last_segment=$4
  backup_id "$anchor_backup" || fail "接起的全量 ID 无效"
  matches "$anchor_name" '^[A-Za-z0-9_]{1,200}$' || fail "接起的全量恢复点名字无效"
  [ -f "/backup/$anchor_backup/postgres/restore-point.json" ] || fail "接起的全量不在仓库里"
  anchor=yes
fi
matches "$from_lsn" "$LSN" || fail "起点 LSN 无效"
matches "$last_segment" "$SEGMENT" || fail "起点所在的段无效"
timeline=$(printf '%s' "$last_segment" | cut -c1-8)

segment_size=$(query "SELECT pg_size_bytes(current_setting('wal_segment_size'))")
matches "$segment_size" '^[0-9]+$' || fail "WAL 段大小无效"
per_log=$((0x100000000 / segment_size))

# 段号 = LSN / 段大小；段名是时间线、段号除以每 4 GB 的段数、余数，各 8 位十六进制。
segment_number() {
  echo $(((0x${1%/*} * 0x100000000 + 0x${1#*/}) / segment_size))
}

segment_name() {
  printf '%s%08X%08X' "$timeline" $(($1 / per_log)) $(($1 % per_log))
}

name_number() {
  echo $((0x$(printf '%s' "$1" | cut -c9-16) * per_log + 0x$(printf '%s' "$1" | cut -c17-24)))
}

# 恢复点和切换分两条语句：中间别的连接写的 WAL 照样在这一段里，下一次从这个恢复点接着读。
name=$(printf 'agent_room_%s' "$AGENT_ROOM_BACKUP_ID" | tr 'TZ-' '___')
set -- $(query "SELECT pg_create_restore_point('$name')::text || ' ' || to_char(clock_timestamp() AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')")
[ "$#" -eq 2 ] || fail "恢复点没有建成"
to_lsn=$1
created_at=$2
matches "$to_lsn" "$LSN" || fail "恢复点 LSN 无效"
matches "$created_at" '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{6}Z$' || fail "恢复点时间无效"
wal_file=$(query "SELECT pg_walfile_name(pg_switch_wal())")
matches "$wal_file" "$SEGMENT" || fail "切换后的 WAL 段无效"
[ "$(printf '%s' "$wal_file" | cut -c1-8)" = "$timeline" ] ||
  fail "时间线变了（$last_segment → $wal_file），要从一套新的全量重新接起"
[ "$wal_file" \> "$last_segment" ] || fail "WAL 没有往前走（$last_segment → $wal_file）"

# 归档命令先写临时文件再改名；以前的归档命令直接 cp，所以还要等到整段的大小。
attempt=0
until [ -f "$archive/$wal_file" ] && [ "$(stat -c %s "$archive/$wal_file")" = "$segment_size" ]; do
  attempt=$((attempt + 1))
  [ "$attempt" -le 180 ] || fail "恢复点所在的段 $wal_file 180 秒内没有归档完"
  sleep 1
done

# 从起点 LSN 所在的段一直到切换后的段，一段不缺地放进核对目录：新归档的从归档目录取，
# 上一次已经收好的从仓库解压。
first=$(segment_number "$from_lsn")
last=$(name_number "$wal_file")
[ "$first" -le "$last" ] || fail "WAL 区间倒置（$from_lsn → $wal_file）"
number=$first
while [ "$number" -le "$last" ]; do
  file=$(segment_name "$number")
  if [ -f "$archive/$file" ]; then
    [ "$(stat -c %s "$archive/$file")" = "$segment_size" ] || fail "归档的段不完整：$file"
    ln "$archive/$file" "$check/$file" 2>/dev/null || cp "$archive/$file" "$check/$file"
  elif [ -f "$store/$file.gz" ]; then
    (cd "$store" && sha256sum -c -s "$file.gz.sha256") || fail "仓库里的段校验不对：$file"
    gzip -dc "$store/$file.gz" >"$check/$file"
  else
    fail "缺少 WAL 段 $file，接不上了"
  fi
  number=$((number + 1))
done

pg_waldump --quiet --path="$check" --timeline="$((0x$timeline))" --start="$from_lsn" --end="$to_lsn" ||
  fail "WAL 读不通（$from_lsn → $to_lsn）：缺段或者坏了"

# 读通了才收：压缩、解压回来比一遍、记下压缩件的 SHA-256，改名以后才算收好。
stored=0
for path in "$check"/*; do
  file=${path##*/}
  [ -f "$archive/$file" ] || continue
  partial="$store/.$file.gz.partial"
  gzip -c "$path" >"$partial"
  gzip -dc "$partial" | cmp -s - "$path" || fail "压缩以后解不回原样：$file"
  digest=$(sha256sum "$partial" | cut -d ' ' -f 1)
  mv "$partial" "$store/$file.gz"
  printf '%s  %s.gz\n' "$digest" "$file" >"$store/.$file.gz.sha256.partial"
  mv "$store/.$file.gz.sha256.partial" "$store/$file.gz.sha256"
  chown "$owner" "$store/$file.gz" "$store/$file.gz.sha256"
  stored=$((stored + 1))
done

partial="$store/.restore-points.log.partial"
if [ -s "$log" ]; then
  cp "$log" "$partial"
else
  : >"$partial"
fi
if [ -n "$anchor" ]; then
  printf '%s anchor %s %s %s %s\n' "$created_at" "$anchor_name" "$from_lsn" "$last_segment" "$anchor_backup" >>"$partial"
fi
printf '%s point %s %s %s -\n' "$created_at" "$name" "$to_lsn" "$wal_file" >>"$partial"
chown "$owner" "$partial"
mv "$partial" "$log"
chown "$owner" "$store"
# 删原文件以前先把仓库里新写的落盘：断电以后不会出现原文件删了、压缩件和记录却没写下去。
sync -f "$log"

# 到切换后的段为止的原文件都收好了（或者在起点以前、早就不要了），删掉；之后归档的留给下一次。
for path in "$archive"/*; do
  [ -f "$path" ] || continue
  file=${path##*/}
  if matches "$file" "$SEGMENT"; then
    [ "$file" \> "$wal_file" ] || rm -f "$path"
  elif matches "$file" '^[0-9A-F]{24}\.[0-9A-F]{8}\.backup$'; then
    [ "$(printf '%s' "$file" | cut -c1-24)" \> "$wal_file" ] || rm -f "$path"
  fi
done
rm -rf "$check"

echo "restore_point=$name lsn=$to_lsn segment=$wal_file stored=$stored"
