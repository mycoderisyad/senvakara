# Read-only. Up to date when the vaultwarden compose project files match these settings and all its containers run.
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
if ! cmp -s "$SV_MODULE_DIR/compose.yml" "$dir/compose.yml"; then
  echo "$dir/compose.yml is missing or differs from the module's."
  exit 10
fi
if [ "$(cat "$dir/.env" 2>/dev/null)" != "$(render_env)" ]; then
  echo "$dir/.env is missing or does not hold these settings yet."
  exit 10
fi
running=$(docker compose -p sv-vaultwarden ps --status running --quiet 2>/dev/null | grep -c . || true)
if [ "$running" -ne 1 ]; then
  echo "Only $running of 1 vaultwarden containers are running."
  exit 10
fi
echo "vaultwarden runs from $dir with these settings."
