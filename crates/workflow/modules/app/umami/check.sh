# Read-only. Up to date when the umami compose project files match these settings and all its containers run.
export LC_ALL=C
dir=/opt/senvakara/apps/umami
q() { printf "'%s'" "$1"; }
if ! docker compose version >/dev/null 2>&1; then
  echo "Docker Engine with the compose plugin is not installed: add the Docker Engine node first."
  exit 2
fi
dbpw=${SV_DB_PASSWORD:-}
secret=${SV_APP_SECRET:-}
if ! [[ "$dbpw" =~ ^[A-Za-z0-9._~-]{8,}$ ]]; then
  echo "The database password needs 8 or more letters, digits, dots, dashes, underscores or tildes."
  exit 1
fi
if [ "${#secret}" -lt 16 ]; then
  echo "The app secret needs at least 16 characters."
  exit 1
fi
case "$secret" in *"'"*|*$'\n'*) echo "The app secret must be one line without a single quote."; exit 1;; esac
render_env() {
  echo "# Written by Senvakara (umami node). Change it there."
  echo "BIND=$SV_BIND"
  echo "PORT=$SV_PORT"
  echo "DB_PASSWORD=$dbpw"
  echo "APP_SECRET=$(q "$secret")"
}
if ! cmp -s "$SV_MODULE_DIR/compose.yml" "$dir/compose.yml"; then
  echo "$dir/compose.yml is missing or differs from the module's."
  exit 10
fi
if [ "$(cat "$dir/.env" 2>/dev/null)" != "$(render_env)" ]; then
  echo "$dir/.env is missing or does not hold these settings yet."
  exit 10
fi
running=$(docker compose -p sv-umami ps --status running --quiet 2>/dev/null | grep -c . || true)
if [ "$running" -ne 2 ]; then
  echo "Only $running of 2 umami containers are running."
  exit 10
fi
echo "umami runs from $dir with these settings."
