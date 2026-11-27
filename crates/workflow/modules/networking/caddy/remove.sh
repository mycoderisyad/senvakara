# Stop Caddy, remove the package (not purge, which would delete /etc/caddy/Caddyfile) and
# its apt repository and key. Keeps /etc/caddy and /var/lib/caddy (certificates) in place.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
lock=(-o DPkg::Lock::Timeout=300)
systemctl disable --now caddy 2>/dev/null || true
apt-get -y -q "${lock[@]}" remove caddy
rm -f /etc/apt/sources.list.d/caddy-stable.sources /etc/apt/sources.list.d/caddy-stable.list /usr/share/keyrings/caddy-stable-archive-keyring.gpg
apt-get -q "${lock[@]}" update || true
echo "Caddy was removed. Its settings stay in /etc/caddy and its certificates in /var/lib/caddy."
