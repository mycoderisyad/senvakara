# Add the MariaDB Foundation apt repository for the chosen series and install mariadb-server.
# The packaged configuration binds to localhost only; this script does not change it.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
lock=(-o DPkg::Lock::Timeout=300)
. /etc/os-release
case "$ID" in
  debian|ubuntu) ;;
  *) echo "The MariaDB repository is only set up for Debian and Ubuntu, this host is $ID."; exit 2 ;;
esac
# deb.mariadb.org publishes no 11.4 packages for Debian 13 (checked against the repository).
if [ "$SV_SERIES" = 11.4 ] && [ "$ID" = debian ] && [ "$VERSION_CODENAME" = trixie ]; then
  echo "MariaDB 11.4 has no packages for Debian 13; choose 11.8 or 12.3."; exit 2
fi

apt-get -q "${lock[@]}" update
apt-get -y -q "${lock[@]}" install ca-certificates curl

key=/etc/apt/keyrings/mariadb-keyring.gpg
src=/etc/apt/sources.list.d/mariadb.sources
install -m 0755 -d /etc/apt/keyrings
if [ ! -s "$key" ]; then
  tmp=$(mktemp)
  curl -fsSL https://supplychain.mariadb.com/mariadb-keyring-2019.gpg -o "$tmp"
  install -m 0644 "$tmp" "$key"
  rm -f "$tmp"
fi

tmp=$(mktemp)
{
  echo "Types: deb"
  echo "URIs: https://deb.mariadb.org/$SV_SERIES/$ID"
  echo "Suites: $VERSION_CODENAME"
  echo "Components: main"
  echo "Signed-By: $key"
} >"$tmp"
if [ "$(cat "$src" 2>/dev/null)" != "$(cat "$tmp")" ]; then
  install -m 0644 "$tmp" "$src"
fi
rm -f "$tmp"

apt-get -q "${lock[@]}" update
apt-get -y -q "${lock[@]}" install mariadb-server
systemctl enable --now mariadb
echo "MariaDB $SV_SERIES is installed and runs as the mariadb service."
