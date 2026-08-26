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

pnpm_store_path=""
if command -v corepack >/dev/null 2>&1; then
  pnpm_store_path="$(corepack pnpm store path --silent 2>/dev/null || true)"
fi

session_root="$(mktemp -d "${TMPDIR:-/tmp}/agent-room-keyring.XXXXXXXX")"
export XDG_DATA_HOME="$session_root/data"
export XDG_RUNTIME_DIR="$session_root/runtime"
mkdir -p "$XDG_DATA_HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$session_root" "$XDG_DATA_HOME" "$XDG_RUNTIME_DIR"

# pnpm 的默认 store 跟随 XDG_DATA_HOME；密钥环隔离不能让已安装的 node_modules
# 突然指向一个空 store，因此固定到进入隔离前的 store 根目录。
if [[ -n "$pnpm_store_path" ]]; then
  export npm_config_store_dir="$(dirname "$pnpm_store_path")"
fi

cleanup() {
  case "$session_root" in
    "${TMPDIR:-/tmp}"/agent-room-keyring.*)
      rm -rf -- "$session_root"
      ;;
    *)
      printf '拒绝清理未经验证的 Secret Service 临时目录：%s。\n' "$session_root" >&2
      ;;
  esac
}
trap cleanup EXIT

keyring_password="$(python3 -c 'import secrets; print(secrets.token_urlsafe(32))')"
daemon_environment="$(
  printf '%s' "$keyring_password" \
    | gnome-keyring-daemon --unlock --components=secrets
)"
unset keyring_password
eval "$daemon_environment"
unset daemon_environment

"$@"
