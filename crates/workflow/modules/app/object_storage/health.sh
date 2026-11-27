# Runs as the signed-in user: is docker active, and does S3 object storage answer on its published port?
wait=${SV_HEALTH_WAIT:-0}
host=$SV_BIND; [ "$host" = "0.0.0.0" ] && host=127.0.0.1
state=unknown; detail="could not read the service state"
if [ ! -d /opt/senvakara/apps/object_storage ]; then
  echo "SV_HEALTH stopped S3 object storage is not installed"
  exit 0
fi
for ((i = 0; i <= wait; i++)); do
  if systemctl is-active --quiet docker 2>/dev/null; then
    if timeout 2 bash -c ": >/dev/tcp/$host/$SV_PORT" 2>/dev/null; then
      state=running; detail="Docker is active and S3 object storage answers on $host:$SV_PORT"; break
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
