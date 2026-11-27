# Stop and remove the Forgejo containers and its compose files; the data volumes are kept.
export LC_ALL=C
dir=/opt/senvakara/apps/forgejo
if [ -f "$dir/compose.yml" ] && docker compose version >/dev/null 2>&1; then
  if [ -f "$dir/.env" ]; then
    docker compose -f "$dir/compose.yml" --env-file "$dir/.env" down --remove-orphans
  else
    docker compose -f "$dir/compose.yml" down --remove-orphans
  fi
fi
rm -f "$dir/compose.yml" "$dir/.env"
rmdir "$dir" 2>/dev/null || true
echo "Forgejo was removed. Its data stays in the Docker volume(s): sv-forgejo_data "
echo "To delete it for good, run as root:"
echo "  docker volume rm sv-forgejo_data"
