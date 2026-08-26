#!/bin/sh
set -eu

read_secret() {
  name="$1"
  path="$2"
  if [ ! -s "$path" ]; then
    echo "缺少 Keycloak Secret：$name。" >&2
    exit 1
  fi
  value=$(cat "$path")
  if [ -z "$value" ]; then
    echo "Keycloak Secret 为空：$name。" >&2
    exit 1
  fi
  printf '%s' "$value"
}

export KC_DB_PASSWORD="$(read_secret KC_DB_PASSWORD /run/secrets/keycloak_db_password)"
export KC_BOOTSTRAP_ADMIN_PASSWORD="$(read_secret KC_BOOTSTRAP_ADMIN_PASSWORD /run/secrets/keycloak_admin_password)"
exec /opt/keycloak/bin/kc.sh "$@"
