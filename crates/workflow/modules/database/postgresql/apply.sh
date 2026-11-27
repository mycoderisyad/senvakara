# Add the PostgreSQL (PGDG) apt repository and install postgresql-<major>.
# Debian packaging creates the "main" cluster and listens on localhost only.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
lock=(-o DPkg::Lock::Timeout=300)
. /etc/os-release
case "$ID" in
  debian|ubuntu) ;;
  *) echo "The PostgreSQL repository is only set up for Debian and Ubuntu, this host is $ID."; exit 2 ;;
esac

apt-get -q "${lock[@]}" update
apt-get -y -q "${lock[@]}" install ca-certificates curl gpg

key=/usr/share/postgresql-common/pgdg/apt.postgresql.org.asc
src=/etc/apt/sources.list.d/pgdg.sources
install -d -m 0755 /usr/share/postgresql-common/pgdg
if [ ! -s "$key" ]; then
  tmp=$(mktemp)
  curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc -o "$tmp"
  fprs=$(gpg --show-keys --with-colons "$tmp" 2>/dev/null || true)
  if ! grep -q '^fpr:::::::::B97B0AFCAA1A47F044F244A07FCC7D46ACCC4CF8:' <<<"$fprs"; then
    rm -f "$tmp"
    echo "The downloaded PostgreSQL key does not have the expected fingerprint."
    exit 1
  fi
  install -m 0644 "$tmp" "$key"
  rm -f "$tmp"
fi

tmp=$(mktemp)
{
  echo "Types: deb"
  echo "URIs: https://apt.postgresql.org/pub/repos/apt"
  echo "Suites: ${VERSION_CODENAME}-pgdg"
  echo "Components: main"
  echo "Signed-By: $key"
} >"$tmp"
if [ "$(cat "$src" 2>/dev/null)" != "$(cat "$tmp")" ]; then
  install -m 0644 "$tmp" "$src"
fi
rm -f "$tmp"

apt-get -q "${lock[@]}" update
apt-get -y -q "${lock[@]}" install "postgresql-$SV_MAJOR"
systemctl enable postgresql
systemctl start "postgresql@$SV_MAJOR-main" 2>/dev/null || systemctl start postgresql
echo "PostgreSQL $SV_MAJOR is installed and listens on localhost port 5432 unless another cluster already uses it."
