# Read-only. Reports whether mariadb is active and answers on TCP 3306.
w=${SV_HEALTH_WAIT:-0}
state=unknown
t=0
while :; do
  state=$(systemctl is-active mariadb 2>/dev/null || true)
  if [ "$state" = active ] && timeout 2 bash -c ': >/dev/tcp/127.0.0.1/3306' 2>/dev/null; then
    echo "SV_HEALTH running MariaDB answers on port 3306"
    exit 0
  fi
  [ "$SECONDS" -lt "$w" ] || break
  sleep 2; t=$((t + 2))
done
case "$state" in
  active) echo "SV_HEALTH degraded mariadb is active but port 3306 does not answer" ;;
  failed) echo "SV_HEALTH stopped mariadb failed" ;;
  inactive) echo "SV_HEALTH stopped mariadb is not running" ;;
  *) echo "SV_HEALTH unknown mariadb state: ${state:-none}" ;;
esac
exit 0
