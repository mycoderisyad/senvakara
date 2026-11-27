# Stop MariaDB, remove its packages and the MariaDB repository and key.
# The packages are removed, not purged, so databases in /var/lib/mysql and settings in /etc/mysql stay.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
lock=(-o DPkg::Lock::Timeout=300)
systemctl disable --now mariadb 2>/dev/null || true
apt-get -y -q "${lock[@]}" remove mariadb-server mariadb-client mariadb-common 2>/dev/null || true
apt-get -y -q "${lock[@]}" autoremove
rm -f /etc/apt/sources.list.d/mariadb.sources /etc/apt/keyrings/mariadb-keyring.gpg
apt-get -q "${lock[@]}" update || true
echo "MariaDB is removed. Databases were left in /var/lib/mysql and settings in /etc/mysql."
