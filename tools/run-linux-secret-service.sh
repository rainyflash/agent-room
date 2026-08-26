#!/usr/bin/env bash
set -euo pipefail

# GitHub 的无头 Linux Runner 没有桌面登录会话；Bridge 仍必须使用真实 Secret Service，
# 因此为每次验收创建独立 D-Bus、XDG 目录和临时登录密钥环。
if (( $# == 0 )); then
  printf '%s\n' '缺少要在 Linux Secret Service 会话中执行的命令。' >&2
  exit 64
fi

for dependency in gnome-keyring-daemon python3; do
  if ! command -v "$dependency" >/dev/null 2>&1; then
    printf '缺少 Linux Secret Service 依赖：%s。\n' "$dependency" >&2
    exit 69
  fi
done

session_root="$(mktemp -d "${TMPDIR:-/tmp}/agent-room-keyring.XXXXXXXX")"
export XDG_DATA_HOME="$session_root/data"
export XDG_RUNTIME_DIR="$session_root/runtime"
mkdir -p "$XDG_DATA_HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$session_root" "$XDG_DATA_HOME" "$XDG_RUNTIME_DIR"

keyring_password="$(python3 -c 'import secrets; print(secrets.token_urlsafe(32))')"
daemon_environment="$(
  printf '%s' "$keyring_password" \
    | gnome-keyring-daemon --unlock --components=secrets
)"
unset keyring_password
eval "$daemon_environment"
unset daemon_environment

"$@"
