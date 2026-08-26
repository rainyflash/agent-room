#!/bin/sh
set -eu

read_secret() {
  path="$1"
  if [ ! -s "$path" ]; then
    echo "缺少 PostgreSQL 初始化 Secret：$path。" >&2
    exit 1
  fi
  cat "$path"
}

agent_room_password=$(read_secret /run/secrets/agent_room_db_migration_password)
agent_room_runtime_password=$(read_secret /run/secrets/agent_room_db_runtime_password)
postgres_metrics_password=$(read_secret /run/secrets/postgres_metrics_password)
synapse_password=$(read_secret /run/secrets/synapse_db_password)
keycloak_password=$(read_secret /run/secrets/keycloak_db_password)

psql --set=ON_ERROR_STOP=1 \
  --username "$POSTGRES_USER" \
  --dbname "$POSTGRES_DB" \
  --set=agent_room_password="$agent_room_password" \
  --set=agent_room_runtime_password="$agent_room_runtime_password" \
  --set=postgres_metrics_password="$postgres_metrics_password" \
  --set=synapse_password="$synapse_password" \
  --set=keycloak_password="$keycloak_password" <<-'SQL'
CREATE ROLE agent_room LOGIN PASSWORD :'agent_room_password' NOSUPERUSER NOCREATEDB NOCREATEROLE;
CREATE ROLE agent_room_runtime LOGIN PASSWORD :'agent_room_runtime_password' NOSUPERUSER NOCREATEDB NOCREATEROLE;
CREATE ROLE agent_room_metrics LOGIN PASSWORD :'postgres_metrics_password' NOSUPERUSER NOCREATEDB NOCREATEROLE;
GRANT pg_monitor TO agent_room_metrics;
CREATE DATABASE agent_room OWNER agent_room;
REVOKE CONNECT ON DATABASE agent_room FROM PUBLIC;
GRANT CONNECT ON DATABASE agent_room TO agent_room_runtime;

CREATE ROLE synapse LOGIN PASSWORD :'synapse_password' NOSUPERUSER NOCREATEDB NOCREATEROLE;
CREATE DATABASE synapse OWNER synapse ENCODING 'UTF8' LC_COLLATE 'C' LC_CTYPE 'C' TEMPLATE template0;

CREATE ROLE identity LOGIN PASSWORD :'keycloak_password' NOSUPERUSER NOCREATEDB NOCREATEROLE;
CREATE DATABASE keycloak OWNER identity;
SQL
