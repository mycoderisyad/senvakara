# Read-only. Up to date when the drop-in holds these settings and the
# running kernel uses them. Keys the kernel does not have (IPv6 turned off)
# are skipped. log_martians is left out on purpose: ufw sets it from
# /etc/ufw/sysctl.conf every time it starts, and its own logging covers it.
export LC_ALL=C
conf=/etc/sysctl.d/90-senvakara-network.conf
settings="net.ipv4.tcp_syncookies=1
net.ipv4.conf.all.accept_redirects=0
net.ipv4.conf.default.accept_redirects=0
net.ipv6.conf.all.accept_redirects=0
net.ipv6.conf.default.accept_redirects=0
net.ipv4.conf.all.secure_redirects=0
net.ipv4.conf.default.secure_redirects=0
net.ipv4.conf.all.send_redirects=0
net.ipv4.conf.default.send_redirects=0
net.ipv4.conf.all.accept_source_route=0
net.ipv4.conf.default.accept_source_route=0
net.ipv6.conf.all.accept_source_route=0
net.ipv6.conf.default.accept_source_route=0
net.ipv4.icmp_echo_ignore_broadcasts=1
net.ipv4.icmp_ignore_bogus_error_responses=1"

want=$( echo "# Written by Senvakara (network kernel settings node)."; printf '%s\n' "$settings" | sed 's/=/ = /' )
if [ "$(cat "$conf" 2>/dev/null)" != "$want" ]; then
  echo "$conf does not hold these settings yet."
  exit 10
fi
while IFS='=' read -r key value; do
  path=/proc/sys/$(printf '%s' "$key" | tr . /)
  [ -e "$path" ] || continue
  if [ "$(cat "$path")" != "$value" ]; then
    echo "$key is $(cat "$path"), not $value."
    exit 10
  fi
done <<<"$settings"
echo "The kernel uses these network settings."
