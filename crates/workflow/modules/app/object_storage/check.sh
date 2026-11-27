# Read-only. Up to date when the compose files and .env match these settings and every container of the project runs.
export LC_ALL=C
dir=/opt/senvakara/apps/object_storage
if ! docker compose version >/dev/null 2>&1; then
  echo "Docker Engine with the compose plugin is not installed: add the Docker Engine node first."
  exit 2
fi

if [ -z "${SV_ACCESS_KEY:-}" ] || [ -z "${SV_SECRET_KEY:-}" ]; then
  echo "The access key and the secret key are both required."; exit 2
fi
# A value with a quote or a line break cannot be written safely into .env.
for v in "${SV_ACCESS_KEY:-}" "${SV_SECRET_KEY:-}"; do
  case $v in
    *\'*|*$'\n'*) echo "A setting contains a single quote or a line break, which this node cannot write into .env. Change it."; exit 1 ;;
  esac
done

render_env() {
  echo "# Written by Senvakara (S3 object storage node). Change it there."
  echo "BIND=$SV_BIND"
  echo "PORT=$SV_PORT"
  echo "ACCESS_KEY='${SV_ACCESS_KEY:-}'"
  echo "SECRET_KEY='${SV_SECRET_KEY:-}'"
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
  echo "Only $have of $want containers of the sv-object_storage project are running."
  exit 10
fi
echo "S3 object storage runs as the sv-object_storage project: S3 API on $SV_BIND:$SV_PORT."
