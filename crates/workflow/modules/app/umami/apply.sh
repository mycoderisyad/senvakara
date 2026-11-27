# Write the umami compose file and settings, then start (or update) its containers.
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
install -d -m 0755 /opt/senvakara/apps "$dir"
install -m 0644 "$SV_MODULE_DIR/compose.yml" "$dir/compose.yml"
umask 077
tmp=$(mktemp "$dir/.env.XXXXXX")
render_env >"$tmp"
install -m 0600 "$tmp" "$dir/.env"
rm -f "$tmp"
docker compose -f "$dir/compose.yml" --env-file "$dir/.env" up -d --pull missing
echo "umami is started from $dir."
