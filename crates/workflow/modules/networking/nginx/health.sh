# Runs as the signed-in user: is nginx active, and does it answer on port 80?
wait=${SV_HEALTH_WAIT:-0}
state=unknown; detail="could not read the service state"
for ((i = 0; i <= wait; i++)); do
  if systemctl is-active --quiet nginx 2>/dev/null; then
    if timeout 2 bash -c ': >/dev/tcp/127.0.0.1/80' 2>/dev/null; then
      state=running; detail="nginx is active and answers on port 80"; break
    fi
    state=degraded; detail="nginx is active but nothing answers on port 80"
  else
    s=$(systemctl is-active nginx 2>/dev/null || true)
    case "$s" in
      failed) state=stopped; detail="nginx has failed" ;;
      inactive) state=stopped; detail="nginx is not running" ;;
      activating) state=degraded; detail="nginx is still starting" ;;
      *) state=unknown; detail="nginx state: ${s:-unknown}" ;;
    esac
  fi
  [ "$SECONDS" -lt "$wait" ] || break
  sleep 1
done
echo "SV_HEALTH $state $detail"
exit 0
