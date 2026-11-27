# Read-only. Up to date when the MariaDB repository for the chosen series is set up,
# mariadb-server of that series is installed and the service runs.
export LC_ALL=C
. /etc/os-release
case "$ID" in
  debian|ubuntu) ;;
  *) echo "The MariaDB repository is only set up for Debian and Ubuntu, this host is $ID."; exit 2 ;;
esac
# deb.mariadb.org publishes no 11.4 packages for Debian 13 (checked against the repository).
if [ "$SV_SERIES" = 11.4 ] && [ "$ID" = debian ] && [ "$VERSION_CODENAME" = trixie ]; then
  echo "MariaDB 11.4 has no packages for Debian 13; choose 11.8 or 12.3."; exit 2
fi

key=/etc/apt/keyrings/mariadb-keyring.gpg
src=/etc/apt/sources.list.d/mariadb.sources
render() {
  echo "Types: deb"
  echo "URIs: https://deb.mariadb.org/$SV_SERIES/$ID"
  echo "Suites: $VERSION_CODENAME"
  echo "Components: main"
  echo "Signed-By: $key"
}

if [ ! -s "$key" ]; then
  echo "The MariaDB signing key is not in place yet."
  exit 10
fi
if [ "$(cat "$src" 2>/dev/null)" != "$(render)" ]; then
  echo "The MariaDB apt repository is not set up for series $SV_SERIES."
  exit 10
fi
ver=$(dpkg-query -W -f='${Version}' mariadb-server 2>/dev/null || true)
pat="^([0-9]+:)?${SV_SERIES//./\\.}\\."
if ! printf '%s' "$ver" | grep -Eq "$pat"; then
  echo "mariadb-server $SV_SERIES is not installed (found: ${ver:-none})."
  exit 10
fi
if ! systemctl is-active --quiet mariadb; then
  echo "The mariadb service is not running."
  exit 10
fi
echo "MariaDB $SV_SERIES is installed and running."
