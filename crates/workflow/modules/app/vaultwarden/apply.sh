# Write the vaultwarden compose file and settings, then start (or update) its containers.
export LC_ALL=C
dir=/opt/senvakara/apps/vaultwarden
q() { printf "'%s'" "$1"; }
if ! docker compose version >/dev/null 2>&1; then
  echo "Docker Engine with the compose plugin is not installed: add the Docker Engine node first."
  exit 2
fi
domain=${SV_DOMAIN:-}
token=${SV_ADMIN_TOKEN:-}
if ! [[ "$domain" =~ ^https://[A-Za-z0-9.-]+(:[0-9]{1,5})?(/[A-Za-z0-9._~/-]*)?$ ]]; then
  echo "The domain must be an https address such as https://vault.example.com."
  exit 1
fi
case "$token" in *"'"*|*$'\n'*) echo "The admin token must be one line without a single quote."; exit 1;; esac
render_env() {
  echo "# Written by Senvakara (vaultwarden node). Change it there."
  echo "BIND=$SV_BIND"
  echo "PORT=$SV_PORT"
  echo "DOMAIN=$(q "$domain")"
  if [ "${SV_SIGNUPS:-0}" = 1 ]; then echo "SIGNUPS_ALLOWED=true"; else echo "SIGNUPS_ALLOWED=false"; fi
  echo "ADMIN_TOKEN=$(q "$token")"
}
install -d -m 0755 /opt/senvakara/apps "$dir"
install -m 0644 "$SV_MODULE_DIR/compose.yml" "$dir/compose.yml"
umask 077
tmp=$(mktemp "$dir/.env.XXXXXX")
render_env >"$tmp"
install -m 0600 "$tmp" "$dir/.env"
rm -f "$tmp"
docker compose -f "$dir/compose.yml" --env-file "$dir/.env" up -d --pull missing
echo "vaultwarden is started from $dir."
