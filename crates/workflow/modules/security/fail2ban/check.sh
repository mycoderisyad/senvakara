# Read-only. Up to date when fail2ban runs with Senvakara's sshd jail as set
# here and the jail is loaded.
export LC_ALL=C
jail=/etc/fail2ban/jail.d/senvakara-sshd.local

installed() { dpkg-query -W -f='${Status}' "$1" 2>/dev/null | grep -q 'install ok installed'; }
if ! installed fail2ban || ! installed python3-systemd; then
  echo "fail2ban or python3-systemd is not installed."
  exit 10
fi

ports=$( { echo "$SV_SESSION_PORT"; sshd -T 2>/dev/null | awk '$1 == "port" { print $2 }'; } | sort -un | paste -sd, -)
render() {
  echo "# Written by Senvakara (fail2ban node). Change it there."
  echo "[sshd]"
  echo "enabled = true"
  echo "backend = systemd"
  echo "port = $ports"
  echo "maxretry = $SV_MAX_RETRY"
  echo "findtime = ${SV_FIND_MINUTES}m"
  echo "bantime = ${SV_BAN_MINUTES}m"
}
if [ "$(cat "$jail" 2>/dev/null)" != "$(render)" ]; then
  echo "$jail does not hold these settings yet."
  exit 10
fi
if ! systemctl is-active --quiet fail2ban; then
  echo "fail2ban is not running."
  exit 10
fi
if ! fail2ban-client status sshd >/dev/null 2>&1; then
  echo "fail2ban runs, but the sshd jail is not loaded."
  exit 10
fi
echo "fail2ban watches SSH sign-ins on port(s) $ports."
