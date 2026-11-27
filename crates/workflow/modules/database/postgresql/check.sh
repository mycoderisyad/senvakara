# Read-only. Up to date when the PGDG repository is set up, postgresql-<major> is
# installed and a cluster of that major is running.
export LC_ALL=C
. /etc/os-release
case "$ID" in
  debian|ubuntu) ;;
  *) echo "The PostgreSQL repository is only set up for Debian and Ubuntu, this host is $ID."; exit 2 ;;
esac

key=/usr/share/postgresql-common/pgdg/apt.postgresql.org.asc
src=/etc/apt/sources.list.d/pgdg.sources
render() {
  echo "Types: deb"
  echo "URIs: https://apt.postgresql.org/pub/repos/apt"
  echo "Suites: ${VERSION_CODENAME}-pgdg"
  echo "Components: main"
  echo "Signed-By: $key"
}

if [ ! -s "$key" ]; then
  echo "The PostgreSQL signing key is not in place yet."
  exit 10
fi
if [ "$(cat "$src" 2>/dev/null)" != "$(render)" ]; then
  echo "The PostgreSQL apt repository is not set up as expected."
  exit 10
fi
if ! dpkg-query -W -f='${Status}' "postgresql-$SV_MAJOR" 2>/dev/null | grep -q 'install ok installed'; then
  echo "postgresql-$SV_MAJOR is not installed."
  exit 10
fi
if ! systemctl is-active --quiet "postgresql@$SV_MAJOR-main"; then
  echo "The PostgreSQL $SV_MAJOR main cluster is not running."
  exit 10
fi
echo "PostgreSQL $SV_MAJOR is installed and running."
