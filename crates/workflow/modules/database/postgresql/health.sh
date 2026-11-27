# Read-only. Reports whether a PostgreSQL cluster is active and answers on TCP 5432.
w=${SV_HEALTH_WAIT:-0}
n=0
tcp=0
t=0
while :; do
  n=$(systemctl list-units 'postgresql@*' --state=active --no-legend 2>/dev/null | wc -l) || n=0
  tcp=0
  if [ "$n" -gt 0 ] && timeout 2 bash -c ': >/dev/tcp/127.0.0.1/5432' 2>/dev/null; then tcp=1; break; fi
  [ "$SECONDS" -lt "$w" ] || break
  sleep 2; t=$((t + 2))
done
if [ "$tcp" = 1 ]; then
  echo "SV_HEALTH running PostgreSQL answers on port 5432"
elif [ "$n" -gt 0 ]; then
  echo "SV_HEALTH degraded PostgreSQL is active but port 5432 does not answer"
elif systemctl cat postgresql >/dev/null 2>&1; then
  echo "SV_HEALTH stopped no PostgreSQL cluster is running"
else
  echo "SV_HEALTH unknown PostgreSQL service not found"
fi
exit 0
