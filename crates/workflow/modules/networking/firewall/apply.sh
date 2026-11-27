# Allow SSH first, then deny incoming by default and enable ufw. Senvakara
# scheduled rollback.sh before this ran; the backup it restores is written
# to $SV_GUARD_DIR first.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C

if ! command -v ufw >/dev/null 2>&1; then
  apt-get -q -o DPkg::Lock::Timeout=300 update
  apt-get -y -q -o DPkg::Lock::Timeout=300 install ufw
fi

install -d -m 0700 "$SV_GUARD_DIR"
if ufw status | grep -q '^Status: active'; then
  echo active >"$SV_GUARD_DIR/was"
else
  echo inactive >"$SV_GUARD_DIR/was"
fi
cp -a /etc/ufw "$SV_GUARD_DIR/etc-ufw"
cp -a /etc/default/ufw "$SV_GUARD_DIR/default-ufw"

ssh_ports=$( { echo "$SV_SESSION_PORT"; sshd -T 2>/dev/null | awk '$1 == "port" { print $2 }'; } | sort -un)
if [ -z "$ssh_ports" ]; then
  echo "Could not tell which port SSH uses; ufw was not changed." >&2
  exit 3
fi
# SSH before anything else (plan WF §4).
for port in $ssh_ports; do
  ufw allow "$port/tcp" comment 'SSH (Senvakara)'
done
for rule in ${SV_ALLOW:-}; do
  ufw allow "$rule" comment 'Senvakara'
done
ufw default deny incoming
ufw default allow outgoing
ufw --force enable
ufw status verbose
