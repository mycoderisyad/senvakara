# Read-only. Up to date when the mailpit compose project files match these settings and all its containers run.
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
if ! cmp -s "$SV_MODULE_DIR/compose.yml" "$dir/compose.yml"; then
  echo "$dir/compose.yml is missing or differs from the module's."
  exit 10
fi
if [ "$(cat "$dir/.env" 2>/dev/null)" != "$(render_env)" ]; then
  echo "$dir/.env is missing or does not hold these settings yet."
  exit 10
fi
running=$(docker compose -p sv-mailpit ps --status running --quiet 2>/dev/null | grep -c . || true)
if [ "$running" -ne 1 ]; then
  echo "Only $running of 1 mailpit containers are running."
  exit 10
fi
echo "mailpit runs from $dir with these settings."
