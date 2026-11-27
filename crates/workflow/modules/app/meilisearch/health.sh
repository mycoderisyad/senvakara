# Answer for the user without sudo: is Docker active and does the published port accept connections.
export LC_ALL=C
wait=${SV_HEALTH_WAIT:-0}
host=$SV_BIND
[ "$host" = "0.0.0.0" ] && host=127.0.0.1
if ! systemctl is-active --quiet docker; then
  echo "SV_HEALTH stopped the Docker service is not running"
  exit 0
fi
ok=0
for ((i = 0; i <= wait; i++)); do
  if timeout 2 bash -c ": >/dev/tcp/$host/$SV_PORT" 2>/dev/null; then ok=1; break; fi
  [ "$SECONDS" -lt "$wait" ] || break
  sleep 1
done
if [ "$ok" -eq 1 ]; then
  echo "SV_HEALTH running Meilisearch answers on $host:$SV_PORT"
else
  echo "SV_HEALTH stopped Docker is running but nothing answers on $host:$SV_PORT"
fi
exit 0
