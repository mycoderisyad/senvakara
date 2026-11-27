# Install fail2ban with journal support, write the sshd jail, restart.
# The systemd backend works whether or not rsyslog writes /var/log/auth.log
# (Debian 12 and later log to the journal only by default).
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
lock=(-o DPkg::Lock::Timeout=300)
apt-get -q "${lock[@]}" update
apt-get -y -q "${lock[@]}" install fail2ban python3-systemd

jail=/etc/fail2ban/jail.d/senvakara-sshd.local
ports=$( { echo "$SV_SESSION_PORT"; sshd -T 2>/dev/null | awk '$1 == "port" { print $2 }'; } | sort -un | paste -sd, -)
tmp=$(mktemp)
{
  echo "# Written by Senvakara (fail2ban node). Change it there."
  echo "[sshd]"
  echo "enabled = true"
  echo "backend = systemd"
  echo "port = $ports"
  echo "maxretry = $SV_MAX_RETRY"
  echo "findtime = ${SV_FIND_MINUTES}m"
  echo "bantime = ${SV_BAN_MINUTES}m"
} >"$tmp"
install -m 0644 "$tmp" "$jail"
rm -f "$tmp"

systemctl enable fail2ban
systemctl restart fail2ban
# The server needs a moment before it answers.
for _ in $(seq 1 20); do
  fail2ban-client ping >/dev/null 2>&1 && break
  sleep 0.5
done
fail2ban-client status sshd
