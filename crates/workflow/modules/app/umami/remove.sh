# Stop and remove the umami containers and its files; the data is kept.
export LC_ALL=C
dir=/opt/senvakara/apps/umami
if docker compose version >/dev/null 2>&1; then
  docker compose -p sv-umami down
fi
rm -f "$dir/compose.yml" "$dir/.env"
rmdir "$dir" 2>/dev/null || true
echo "Umami and its database are stopped and removed. The statistics were kept in the Docker volume sv-umami_db-data."
echo "To delete them for good, run as root: docker volume rm sv-umami_db-data"
echo "Note: a later install needs the same database password, or the kept database will refuse the new one."
