# Read-only. Up to date when Senvakara's drop-in holds the wanted settings,
# sshd's effective configuration (sshd -T) agrees, and a new port is
# listening.
export LC_ALL=C
conf=/etc/ssh/sshd_config.d/00-senvakara-hardening.conf

render() {
  echo "# Written by Senvakara (SSH hardening node). Change it there."
  [ -n "${SV_PORT:-}" ] && echo "Port $SV_PORT"
  echo "PermitRootLogin $SV_PERMIT_ROOT_LOGIN"
  if [ "$SV_DISABLE_PASSWORDS" = 1 ]; then
    echo "PasswordAuthentication no"
    echo "KbdInteractiveAuthentication no"
  fi
  echo "PermitEmptyPasswords no"
}

if [ "$(cat "$conf" 2>/dev/null)" != "$(render)" ]; then
  echo "$conf does not hold these settings yet."
  exit 10
fi

# sshd checks its configuration against the privilege separation folder.
[ -d /run/sshd ] || { echo "sshd is not running (/run/sshd is missing)." >&2; exit 2; }
effective=$(sshd -T) || { echo "sshd -T could not read the configuration." >&2; exit 2; }
value() { printf '%s\n' "$effective" | awk -v k="$1" '$1 == k { print $2; exit }'; }

want_root=$SV_PERMIT_ROOT_LOGIN
[ "$want_root" = prohibit-password ] && [ "$(value permitrootlogin)" = without-password ] && want_root=without-password
if [ "$(value permitrootlogin)" != "$want_root" ]; then
  echo "sshd still uses PermitRootLogin $(value permitrootlogin): another file sets it first."
  exit 10
fi
if [ "$SV_DISABLE_PASSWORDS" = 1 ]; then
  if [ "$(value passwordauthentication)" != no ] || [ "$(value kbdinteractiveauthentication)" != no ]; then
    echo "sshd still accepts passwords: another file sets it first."
    exit 10
  fi
fi
if [ -n "${SV_PORT:-}" ]; then
  if ! printf '%s\n' "$effective" | awk '$1 == "port" { print $2 }' | grep -qx "$SV_PORT"; then
    echo "sshd is not configured for port $SV_PORT."
    exit 10
  fi
  if [ -z "$(ss -Hltn "sport = :$SV_PORT")" ]; then
    echo "Nothing listens on port $SV_PORT yet."
    exit 10
  fi
fi
echo "sshd uses these settings."
