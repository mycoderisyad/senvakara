# Write the mailpit compose file and settings, then start (or update) its containers.
export LC_ALL=C
dir=/opt/senvakara/apps/mailpit
if ! docker compose version >/dev/null 2>&1; then
  echo "Docker Engine with the compose plugin is not installed: add the Docker Engine node first."
  exit 2
fi
if [ "$SV_PORT" = "$SV_SMTP_PORT" ]; then
  echo "The web port and the SMTP port must differ."
  exit 1
fi
render_env() {
  echo "# Written by Senvakara (mailpit node). Change it there."
  echo "BIND=$SV_BIND"
  echo "PORT=$SV_PORT"
  echo "SMTP_PORT=$SV_SMTP_PORT"
  echo "MAX_MESSAGES=$SV_MAX_MESSAGES"
}
install -d -m 0755 /opt/senvakara/apps "$dir"
install -m 0644 "$SV_MODULE_DIR/compose.yml" "$dir/compose.yml"
umask 077
tmp=$(mktemp "$dir/.env.XXXXXX")
render_env >"$tmp"
install -m 0600 "$tmp" "$dir/.env"
rm -f "$tmp"
docker compose -f "$dir/compose.yml" --env-file "$dir/.env" up -d --pull missing
echo "mailpit is started from $dir."
