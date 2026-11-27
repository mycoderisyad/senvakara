# Runs as the signed-in user: is docker active, and does Grafana + Prometheus answer on its published port?
wait=${SV_HEALTH_WAIT:-0}
host=$SV_BIND; [ "$host" = "0.0.0.0" ] && host=127.0.0.1
state=unknown; detail="could not read the service state"
if [ ! -d /opt/senvakara/apps/monitoring ]; then
  echo "SV_HEALTH stopped Grafana + Prometheus is not installed"
  exit 0
fi
for ((i = 0; i <= wait; i++)); do
  if systemctl is-active --quiet docker 2>/dev/null; then
    if timeout 2 bash -c ": >/dev/tcp/$host/$SV_PORT" 2>/dev/null; then
      state=running; detail="Docker is active and Grafana + Prometheus answers on $host:$SV_PORT"; break
    fi
    state=stopped; detail="Docker is active but nothing answers on $host:$SV_PORT"
  else
    s=$(systemctl is-active docker 2>/dev/null || true)
    case "$s" in
      inactive) state=stopped; detail="Docker is not running" ;;
      failed) state=stopped; detail="Docker has failed" ;;
      activating) state=degraded; detail="Docker is still starting" ;;
      *) state=unknown; detail="Docker state: ${s:-unknown}" ;;
    esac
  fi
  [ "$SECONDS" -lt "$wait" ] || break
  sleep 1
done
echo "SV_HEALTH $state $detail"
exit 0
