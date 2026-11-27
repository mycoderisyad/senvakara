# Stop and disable wg-quick@wg0 and purge the WireGuard packages. Keeps everything in
# /etc/wireguard (the server key and wg0.conf) so a later install reuses the same key.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
lock=(-o DPkg::Lock::Timeout=300)
systemctl disable --now wg-quick@wg0 2>/dev/null || true
apt-get -y -q "${lock[@]}" remove wireguard wireguard-tools
echo "WireGuard was removed. The server key and wg0.conf stay in /etc/wireguard."
