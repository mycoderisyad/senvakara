# Stop Redis, remove (not purge) its packages and remove the Redis repository and key.
# Saved data stays in /var/lib/redis.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
lock=(-o DPkg::Lock::Timeout=300)
systemctl disable --now redis-server 2>/dev/null || true
apt-get -y -q "${lock[@]}" remove redis redis-server redis-tools 2>/dev/null || true
apt-get -y -q "${lock[@]}" autoremove
rm -f /etc/apt/sources.list.d/redis.sources /etc/apt/keyrings/redis.gpg
apt-get -q "${lock[@]}" update || true
echo "Redis is removed. Saved data was left in /var/lib/redis."
