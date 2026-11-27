# Write the meilisearch compose file and settings, then start (or update) its containers.
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
install -d -m 0755 /opt/senvakara/apps "$dir"
install -m 0644 "$SV_MODULE_DIR/compose.yml" "$dir/compose.yml"
umask 077
tmp=$(mktemp "$dir/.env.XXXXXX")
render_env >"$tmp"
install -m 0600 "$tmp" "$dir/.env"
rm -f "$tmp"
docker compose -f "$dir/compose.yml" --env-file "$dir/.env" up -d --pull missing
echo "meilisearch is started from $dir."
