# Read-only. Up to date when the meilisearch compose project files match these settings and all its containers run.
export LC_ALL=C
dir=/opt/senvakara/apps/meilisearch
q() { printf "'%s'" "$1"; }
if ! docker compose version >/dev/null 2>&1; then
  echo "Docker Engine with the compose plugin is not installed: add the Docker Engine node first."
  exit 2
fi
key=${SV_MASTER_KEY:-}
if [ "${#key}" -lt 16 ]; then
  echo "The master key needs at least 16 bytes."
  exit 1
fi
case "$key" in *"'"*|*$'\n'*) echo "The master key must be one line without a single quote."; exit 1;; esac
render_env() {
  echo "# Written by Senvakara (meilisearch node). Change it there."
  echo "BIND=$SV_BIND"
  echo "PORT=$SV_PORT"
  echo "MASTER_KEY=$(q "$key")"
}
if ! cmp -s "$SV_MODULE_DIR/compose.yml" "$dir/compose.yml"; then
  echo "$dir/compose.yml is missing or differs from the module's."
  exit 10
fi
if [ "$(cat "$dir/.env" 2>/dev/null)" != "$(render_env)" ]; then
  echo "$dir/.env is missing or does not hold these settings yet."
  exit 10
fi
running=$(docker compose -p sv-meilisearch ps --status running --quiet 2>/dev/null | grep -c . || true)
if [ "$running" -ne 1 ]; then
  echo "Only $running of 1 meilisearch containers are running."
  exit 10
fi
echo "meilisearch runs from $dir with these settings."
