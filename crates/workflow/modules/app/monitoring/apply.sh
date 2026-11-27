# Install the compose files and .env, then start Grafana + Prometheus with docker compose.
export LC_ALL=C
dir=/opt/senvakara/apps/monitoring
if ! docker compose version >/dev/null 2>&1; then
  echo "Docker Engine with the compose plugin is not installed: add the Docker Engine node first."
  exit 2
fi

if [ -z "${SV_ADMIN_PASSWORD:-}" ]; then
  echo "The Grafana admin password is required."; exit 2
fi
# A value with a quote or a line break cannot be written safely into .env.
for v in "${SV_ADMIN_PASSWORD:-}"; do
  case $v in
    *\'*|*$'\n'*) echo "A setting contains a single quote or a line break, which this node cannot write into .env. Change it."; exit 1 ;;
  esac
done

render_env() {
  echo "# Written by Senvakara (Grafana + Prometheus node). Change it there."
  echo "BIND=$SV_BIND"
  echo "PORT=$SV_PORT"
  echo "RETENTION_DAYS=$SV_RETENTION_DAYS"
  echo "ADMIN_PASSWORD='${SV_ADMIN_PASSWORD:-}'"
}

mkdir -p /opt/senvakara/apps "$dir"
chmod 0755 /opt/senvakara /opt/senvakara/apps "$dir"
install -m 0644 "$SV_MODULE_DIR/compose.yml" "$dir/compose.yml"
# Single-file mounts keep the old file until the containers restart.
changed=0
for f in prometheus.yml grafana-datasource.yml; do
  cmp -s "$SV_MODULE_DIR/$f" "$dir/$f" || changed=1
  install -m 0644 "$SV_MODULE_DIR/$f" "$dir/$f"
done
umask 077
tmp=$(mktemp "$dir/.env.XXXXXX")
render_env >"$tmp"
install -m 0600 "$tmp" "$dir/.env"
rm -f "$tmp"

docker compose -f "$dir/compose.yml" --env-file "$dir/.env" up -d --pull missing --remove-orphans
if [ "$changed" = 1 ]; then
  docker compose -f "$dir/compose.yml" --env-file "$dir/.env" restart
fi
# Give the containers a moment to start.
want=$(docker compose -f "$dir/compose.yml" --env-file "$dir/.env" config --services | wc -l)
for _ in $(seq 1 30); do
  have=$(docker compose -f "$dir/compose.yml" --env-file "$dir/.env" ps --status running --quiet 2>/dev/null | wc -l) || have=0
  [ "$have" -ge "$want" ] && break
  sleep 1
done
docker compose -f "$dir/compose.yml" --env-file "$dir/.env" ps
