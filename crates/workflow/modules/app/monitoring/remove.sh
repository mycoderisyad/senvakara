# Stop and remove the Grafana + Prometheus containers and its compose files; the data volumes are kept.
export LC_ALL=C
dir=/opt/senvakara/apps/monitoring
if [ -f "$dir/compose.yml" ] && docker compose version >/dev/null 2>&1; then
  if [ -f "$dir/.env" ]; then
    docker compose -f "$dir/compose.yml" --env-file "$dir/.env" down --remove-orphans
  else
    docker compose -f "$dir/compose.yml" down --remove-orphans
  fi
fi
rm -f "$dir/compose.yml" "$dir/.env" "$dir/prometheus.yml" "$dir/grafana-datasource.yml"
rmdir "$dir" 2>/dev/null || true
echo "Grafana + Prometheus was removed. Its data stays in the Docker volume(s): sv-monitoring_prometheus_data sv-monitoring_grafana_data "
echo "To delete it for good, run as root:"
echo "  docker volume rm sv-monitoring_prometheus_data"
echo "  docker volume rm sv-monitoring_grafana_data"
