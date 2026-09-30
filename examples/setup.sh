#!/usr/bin/env bash
# Creates .env with fresh secrets, starts Postgres and Garage, prepares the storage and starts Bulut.
# Usage: ./setup.sh            (HOST_PORT=9000 ./setup.sh to use another port)
set -euo pipefail
cd "$(dirname "$0")"

if [ -e .env ]; then
    echo ".env already exists. To start over: docker compose down -v && rm .env" >&2
    exit 1
fi
command -v openssl >/dev/null || { echo "openssl is needed to make the secrets" >&2; exit 1; }

port="${HOST_PORT:-8080}"
umask 077
cat > .env <<EOF
HOST_PORT=${port}
BASE_URL=http://localhost:${port}
POSTGRES_PASSWORD=$(openssl rand -hex 24)
GARAGE_RPC_SECRET=$(openssl rand -hex 32)
S3_ACCESS_KEY_ID=GK$(openssl rand -hex 12)
S3_SECRET_ACCESS_KEY=$(openssl rand -hex 32)
EOF
# shellcheck disable=SC1091
. ./.env

docker compose up -d postgres garage

echo "Waiting for Garage..."
for _ in $(seq 1 60); do
    if docker compose exec -T garage /garage status >/dev/null 2>&1; then break; fi
    sleep 1
done

# A fresh Garage node holds no data until it has a layout. The key is imported so that Bulut gets
# the credentials generated above. Bulut creates its own bucket when it starts.
node=$(docker compose exec -T garage /garage node id -q | cut -d@ -f1)
docker compose exec -T garage /garage layout assign -z dc1 -c 1G "$node"
docker compose exec -T garage /garage layout apply --version 1
docker compose exec -T garage /garage key import --yes -n bulut "$S3_ACCESS_KEY_ID" "$S3_SECRET_ACCESS_KEY"
docker compose exec -T garage /garage key allow --create-bucket bulut

docker compose up -d bulut

cat <<EOF

Bulut is starting on http://localhost:${port}

Bulut has no authentication. Before anyone else can reach it, put a reverse proxy in front that
requires basic auth for people and a bearer token for agents and REST clients.
EOF
