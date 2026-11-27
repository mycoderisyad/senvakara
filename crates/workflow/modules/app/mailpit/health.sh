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
  if timeout 2 bash -c ": >/dev/tcp/$host/$SV_SMTP_PORT" 2>/dev/null; then
    echo "SV_HEALTH running Mailpit web on $host:$SV_PORT and SMTP on $host:$SV_SMTP_PORT"
  else
    echo "SV_HEALTH degraded Mailpit web answers but SMTP does not on $host:$SV_SMTP_PORT"
  fi
else
  echo "SV_HEALTH stopped Docker is running but nothing answers on $host:$SV_PORT"
fi
exit 0
