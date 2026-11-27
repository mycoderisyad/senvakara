# Runs as the signed-in user: is the Senvakara Cloudflare Tunnel service active?
wait=${SV_HEALTH_WAIT:-0}
state=unknown; detail="could not read the service state"
for ((i = 0; i <= wait; i++)); do
  s=$(systemctl is-active senvakara-cloudflared 2>/dev/null || true)
  case "$s" in
    active) state=running; detail="the tunnel service is active"; break ;;
    activating) state=degraded; detail="the tunnel service is still starting" ;;
    failed) state=stopped; detail="the tunnel service has failed" ;;
    inactive) state=stopped; detail="the tunnel service is not running" ;;
    *) state=unknown; detail="tunnel service state: ${s:-unknown}" ;;
  esac
  [ "$SECONDS" -lt "$wait" ] || break
  sleep 1
done
echo "SV_HEALTH $state $detail"
exit 0
