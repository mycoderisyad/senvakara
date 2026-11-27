# Stop and remove the vaultwarden containers and its files; the data is kept.
export LC_ALL=C
dir=/opt/senvakara/apps/vaultwarden
if docker compose version >/dev/null 2>&1; then
  docker compose -p sv-vaultwarden down
fi
rm -f "$dir/compose.yml" "$dir/.env"
rmdir "$dir" 2>/dev/null || true
echo "Vaultwarden is stopped and removed. Its data (vaults, attachments) was kept in the Docker volume sv-vaultwarden_data."
echo "To delete it for good, run as root: docker volume rm sv-vaultwarden_data"
