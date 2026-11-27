# Read-only. Up to date when the package lists were refreshed by this module
# within the last hour and apt has nothing to install from them. Refreshing
# the lists writes to /var/lib/apt, so that is left to apply.sh.
export LC_ALL=C
stamp=/var/lib/senvakara/apt-updated

if [ -z "$(find "$stamp" -mmin -60 2>/dev/null)" ]; then
  echo "The package lists were not refreshed in the last hour."
  exit 10
fi

mode=upgrade
[ "$SV_FULL_UPGRADE" = 1 ] && mode=dist-upgrade
plan=$(apt-get -s -q "$mode")
pending=$(printf '%s\n' "$plan" | grep -c '^Inst ' || true)
echo "SV_FACT pending_updates=$pending"
if [ "$pending" -gt 0 ]; then
  echo "$pending package(s) can be updated."
  exit 10
fi
echo "No updates pending."
