# Stop and remove nginx and its apt repository, pin and key. Keeps /etc/nginx (your sites)
# and /var/log/nginx in place.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
lock=(-o DPkg::Lock::Timeout=300)
systemctl disable --now nginx 2>/dev/null || true
apt-get -y -q "${lock[@]}" remove nginx
rm -f /etc/apt/sources.list.d/nginx.sources /etc/apt/sources.list.d/nginx.list /etc/apt/preferences.d/99nginx /usr/share/keyrings/nginx-archive-keyring.gpg
apt-get -q "${lock[@]}" update || true
echo "nginx was removed. Its settings stay in /etc/nginx and its logs in /var/log/nginx."
