# Read-only. Reports whether redis-server is active and answers on TCP 6379.
w=${SV_HEALTH_WAIT:-0}
state=unknown
t=0
while :; do
  state=$(systemctl is-active redis-server 2>/dev/null || true)
  if [ "$state" = active ] && timeout 2 bash -c ': >/dev/tcp/127.0.0.1/6379' 2>/dev/null; then
    echo "SV_HEALTH running Redis answers on port 6379"
    exit 0
  fi
  [ "$SECONDS" -lt "$w" ] || break
  sleep 2; t=$((t + 2))
done
case "$state" in
  active) echo "SV_HEALTH degraded redis-server is active but port 6379 does not answer" ;;
  failed) echo "SV_HEALTH stopped redis-server failed" ;;
  inactive) echo "SV_HEALTH stopped redis-server is not running" ;;
  *) echo "SV_HEALTH unknown redis-server state: ${state:-none}" ;;
esac
exit 0
