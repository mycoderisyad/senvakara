# Read-only. Reports whether the docker service is active.
w=${SV_HEALTH_WAIT:-0}
state=$(systemctl is-active docker 2>/dev/null || true)
t=0
while [ "$state" != active ] && [ "$SECONDS" -lt "$w" ]; do
  sleep 2; t=$((t + 2))
  state=$(systemctl is-active docker 2>/dev/null || true)
done
case "$state" in
  active) echo "SV_HEALTH running docker service is active" ;;
  failed) echo "SV_HEALTH stopped docker service failed" ;;
  inactive) echo "SV_HEALTH stopped docker service is not running" ;;
  *) echo "SV_HEALTH unknown docker service state: ${state:-none}" ;;
esac
exit 0
