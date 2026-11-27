# Stop and remove the adminer containers and its files; the data is kept.
export LC_ALL=C
dir=/opt/senvakara/apps/adminer
if docker compose version >/dev/null 2>&1; then
  docker compose -p sv-adminer down
fi
rm -f "$dir/compose.yml" "$dir/.env"
rmdir "$dir" 2>/dev/null || true
echo "Adminer is stopped and removed. It keeps no data, so nothing was left behind."
