# Stop PostgreSQL, remove the server package of the chosen major (not purge: purging can delete
# the cluster) and remove the PGDG repository. Databases stay in /var/lib/postgresql and the
# configuration in /etc/postgresql.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
lock=(-o DPkg::Lock::Timeout=300)
systemctl stop "postgresql@$SV_MAJOR-main" 2>/dev/null || true
apt-get -y -q "${lock[@]}" remove "postgresql-$SV_MAJOR" "postgresql-client-$SV_MAJOR" 2>/dev/null || true
apt-get -y -q "${lock[@]}" autoremove
rm -f /etc/apt/sources.list.d/pgdg.sources /usr/share/postgresql-common/pgdg/apt.postgresql.org.asc
apt-get -q "${lock[@]}" update || true
echo "PostgreSQL $SV_MAJOR is removed. Databases were left in /var/lib/postgresql/$SV_MAJOR and settings in /etc/postgresql/$SV_MAJOR."
