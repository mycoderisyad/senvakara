# Read-only. Up to date when the time zone is set and NTP is switched on
# (systemd-timesyncd) or chrony runs. Whether the clock is already in sync
# shows in the host's Security section; it can take a minute after a change.
export LC_ALL=C
zone=$SV_TIMEZONE
if ! printf '%s' "$zone" | grep -qE '^[A-Za-z0-9_+-]+(/[A-Za-z0-9_+-]+)*$' || [ ! -f "/usr/share/zoneinfo/$zone" ]; then
  echo "$zone is not a time zone this host knows." >&2
  exit 2
fi
if [ "$(timedatectl show -p Timezone --value)" != "$zone" ]; then
  echo "The time zone is $(timedatectl show -p Timezone --value), not $zone."
  exit 10
fi
if systemctl is-active --quiet chrony; then
  echo "Time zone $zone; chrony keeps the clock in sync."
  exit 0
fi
if [ "$(timedatectl show -p NTP --value)" != yes ]; then
  echo "NTP is off."
  exit 10
fi
echo "Time zone $zone; NTP is on."
