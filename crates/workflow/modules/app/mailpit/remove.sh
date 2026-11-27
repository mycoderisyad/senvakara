# Stop and remove the mailpit containers and its files; the data is kept.
export LC_ALL=C
dir=/opt/senvakara/apps/mailpit
if docker compose version >/dev/null 2>&1; then
  docker compose -p sv-mailpit down
fi
rm -f "$dir/compose.yml" "$dir/.env"
rmdir "$dir" 2>/dev/null || true
echo "Mailpit is stopped and removed. Its stored messages were kept in the Docker volume sv-mailpit_data."
echo "To delete them for good, run as root: docker volume rm sv-mailpit_data"
