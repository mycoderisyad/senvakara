# Read-only. Up to date when ufw is active, denies incoming by default and
# has an allow rule for every SSH port and every port asked for.
export LC_ALL=C

if ! command -v ufw >/dev/null 2>&1; then
  echo "ufw is not installed."
  exit 10
fi
status=$(ufw status verbose)
if ! printf '%s\n' "$status" | grep -q '^Status: active'; then
  echo "ufw is not active."
  exit 10
fi
if ! printf '%s\n' "$status" | grep -q 'deny (incoming)'; then
  echo "ufw does not deny incoming connections by default."
  exit 10
fi

# The session's port and every port sshd is configured for.
ssh_ports=$( { echo "$SV_SESSION_PORT"; sshd -T 2>/dev/null | awk '$1 == "port" { print $2 }'; } | sort -un)
added=$(ufw show added)
for rule in $(printf '%s/tcp\n' $ssh_ports) ${SV_ALLOW:-}; do
  if ! printf '%s\n' "$added" | grep -qE "^ufw (allow|limit) $rule( |\$)"; then
    echo "No allow rule for $rule yet."
    exit 10
  fi
done
echo "ufw is active and allows: $(printf '%s/tcp ' $ssh_ports)${SV_ALLOW:-}"
