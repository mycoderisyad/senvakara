# Install the compose files and .env, then start Uptime Kuma with docker compose.
export LC_ALL=C
dir=/opt/senvakara/apps/uptime_kuma
if ! docker compose version >/dev/null 2>&1; then
  echo "Docker Engine with the compose plugin is not installed: add the Docker Engine node first."
  exit 2
fi


render_env() {
  echo "# Written by Senvakara (Uptime Kuma node). Change it there."
  echo "BIND=$SV_BIND"
  echo "PORT=$SV_PORT"
}

mkdir -p /opt/senvakara/apps "$dir"
chmod 0755 /opt/senvakara /opt/senvakara/apps "$dir"
install -m 0644 "$SV_MODULE_DIR/compose.yml" "$dir/compose.yml"
umask 077
tmp=$(mktemp "$dir/.env.XXXXXX")
render_env >"$tmp"
install -m 0600 "$tmp" "$dir/.env"
rm -f "$tmp"

docker compose -f "$dir/compose.yml" --env-file "$dir/.env" up -d --pull missing --remove-orphans
# Give the containers a moment to start.
want=$(docker compose -f "$dir/compose.yml" --env-file "$dir/.env" config --services | wc -l)
for _ in $(seq 1 30); do
  have=$(docker compose -f "$dir/compose.yml" --env-file "$dir/.env" ps --status running --quiet 2>/dev/null | wc -l) || have=0
  [ "$have" -ge "$want" ] && break
  sleep 1
done
docker compose -f "$dir/compose.yml" --env-file "$dir/.env" ps
