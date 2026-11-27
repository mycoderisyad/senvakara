# Read-only. Up to date when the adminer compose project files match these settings and all its containers run.
export LC_ALL=C
dir=/opt/senvakara/apps/adminer
if ! docker compose version >/dev/null 2>&1; then
  echo "Docker Engine with the compose plugin is not installed: add the Docker Engine node first."
  exit 2
fi
render_env() {
  echo "# Written by Senvakara (adminer node). Change it there."
  echo "BIND=$SV_BIND"
  echo "PORT=$SV_PORT"
}
if ! cmp -s "$SV_MODULE_DIR/compose.yml" "$dir/compose.yml"; then
  echo "$dir/compose.yml is missing or differs from the module's."
  exit 10
fi
if [ "$(cat "$dir/.env" 2>/dev/null)" != "$(render_env)" ]; then
  echo "$dir/.env is missing or does not hold these settings yet."
  exit 10
fi
running=$(docker compose -p sv-adminer ps --status running --quiet 2>/dev/null | grep -c . || true)
if [ "$running" -ne 1 ]; then
  echo "Only $running of 1 adminer containers are running."
  exit 10
fi
echo "adminer runs from $dir with these settings."
