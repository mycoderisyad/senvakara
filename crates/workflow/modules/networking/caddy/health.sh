# Runs as the signed-in user: is caddy active, and does it answer on port 80 or 443?
wait=${SV_HEALTH_WAIT:-0}
state=unknown; detail="could not read the service state"
for ((i = 0; i <= wait; i++)); do
  if systemctl is-active --quiet caddy 2>/dev/null; then
    if timeout 2 bash -c ': >/dev/tcp/127.0.0.1/80' 2>/dev/null || timeout 2 bash -c ': >/dev/tcp/127.0.0.1/443' 2>/dev/null; then
      state=running; detail="caddy is active and answers on port 80 or 443"; break
    fi
    state=degraded; detail="caddy is active but nothing answers on port 80 or 443"
  else
    s=$(systemctl is-active caddy 2>/dev/null || true)
    case "$s" in
      failed) state=stopped; detail="caddy has failed" ;;
      inactive) state=stopped; detail="caddy is not running" ;;
      activating) state=degraded; detail="caddy is still starting" ;;
      *) state=unknown; detail="caddy state: ${s:-unknown}" ;;
    esac
  fi
  [ "$SECONDS" -lt "$wait" ] || break
  sleep 1
done
echo "SV_HEALTH $state $detail"
exit 0
