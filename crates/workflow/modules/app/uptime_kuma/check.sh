# Read-only. Up to date when the compose files and .env match these settings and every container of the project runs.
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

if ! cmp -s "$SV_MODULE_DIR/compose.yml" "$dir/compose.yml"; then
  echo "$dir/compose.yml is missing or differs from the module's."
  exit 10
fi
if [ "$(cat "$dir/.env" 2>/dev/null)" != "$(render_env)" ]; then
  echo "$dir/.env does not hold these settings yet."
  exit 10
fi
want=$(docker compose -f "$dir/compose.yml" --env-file "$dir/.env" config --services | wc -l)
have=$(docker compose -f "$dir/compose.yml" --env-file "$dir/.env" ps --status running --quiet 2>/dev/null | wc -l) || have=0
if [ "$have" -lt "$want" ]; then
  echo "Only $have of $want containers of the sv-uptime_kuma project are running."
  exit 10
fi
echo "Uptime Kuma runs as the sv-uptime_kuma project: listening on $SV_BIND:$SV_PORT."
