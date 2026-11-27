# Stop and remove the meilisearch containers and its files; the data is kept.
export LC_ALL=C
dir=/opt/senvakara/apps/meilisearch
if docker compose version >/dev/null 2>&1; then
  docker compose -p sv-meilisearch down
fi
rm -f "$dir/compose.yml" "$dir/.env"
rmdir "$dir" 2>/dev/null || true
echo "Meilisearch is stopped and removed. Its indexes were kept in the Docker volume sv-meilisearch_data."
echo "To delete them for good, run as root: docker volume rm sv-meilisearch_data"
