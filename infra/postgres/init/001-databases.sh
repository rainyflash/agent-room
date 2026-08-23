#!/bin/sh
set -eu

psql --set=ON_ERROR_STOP=1 \
  --username "$POSTGRES_USER" \
  --dbname "$POSTGRES_DB" \
  --set=agent_room_password="$AGENT_ROOM_DB_PASSWORD" \
  --set=synapse_password="$SYNAPSE_DB_PASSWORD" \
  --set=keycloak_password="$KEYCLOAK_DB_PASSWORD" <<-'SQL'
CREATE ROLE agent_room LOGIN PASSWORD :'agent_room_password';
CREATE DATABASE agent_room OWNER agent_room;

CREATE ROLE synapse LOGIN PASSWORD :'synapse_password';
CREATE DATABASE synapse OWNER synapse ENCODING 'UTF8' LC_COLLATE 'C' LC_CTYPE 'C' TEMPLATE template0;

CREATE ROLE identity LOGIN PASSWORD :'keycloak_password';
CREATE DATABASE keycloak OWNER identity;
SQL
