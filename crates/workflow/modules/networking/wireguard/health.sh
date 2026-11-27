# Runs as the signed-in user: is wg-quick@wg0 active and does the wg0 interface exist?
wait=${SV_HEALTH_WAIT:-0}
state=unknown; detail="could not read the service state"
for ((i = 0; i <= wait; i++)); do
  s=$(systemctl is-active wg-quick@wg0 2>/dev/null || true)
  case "$s" in
    active)
      if ip link show wg0 >/dev/null 2>&1; then
        state=running; detail="wg-quick@wg0 is active and wg0 is up"; break
      fi
      state=degraded; detail="wg-quick@wg0 is active but the wg0 interface is missing" ;;
    activating) state=degraded; detail="wg-quick@wg0 is still starting" ;;
    failed) state=stopped; detail="wg-quick@wg0 has failed" ;;
    inactive) state=stopped; detail="wg-quick@wg0 is not running" ;;
    *) state=unknown; detail="wg-quick@wg0 state: ${s:-unknown}" ;;
  esac
  [ "$SECONDS" -lt "$wait" ] || break
  sleep 1
done
echo "SV_HEALTH $state $detail"
exit 0
