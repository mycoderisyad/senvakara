# Set the time zone, then switch NTP on unless chrony already keeps time.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
zone=$SV_TIMEZONE
if ! printf '%s' "$zone" | grep -qE '^[A-Za-z0-9_+-]+(/[A-Za-z0-9_+-]+)*$' || [ ! -f "/usr/share/zoneinfo/$zone" ]; then
  echo "$zone is not a time zone this host knows." >&2
  exit 2
fi
timedatectl set-timezone "$zone"

if systemctl is-active --quiet chrony; then
  echo "chrony runs and keeps the clock in sync; left as it is."
  exit 0
fi
if ! dpkg-query -W -f='${Status}' systemd-timesyncd 2>/dev/null | grep -q 'install ok installed'; then
  apt-get -q -o DPkg::Lock::Timeout=300 update
  apt-get -y -q -o DPkg::Lock::Timeout=300 install systemd-timesyncd
fi
timedatectl set-ntp true
timedatectl show -p Timezone -p NTP -p NTPSynchronized
