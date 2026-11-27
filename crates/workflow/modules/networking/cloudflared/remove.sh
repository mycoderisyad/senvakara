# Stop the Senvakara tunnel service, purge cloudflared and remove its apt repository and key.
# The tunnel itself stays in your Cloudflare account; delete it there if you no longer want it.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
lock=(-o DPkg::Lock::Timeout=300)
systemctl disable --now senvakara-cloudflared 2>/dev/null || true
rm -f /etc/systemd/system/senvakara-cloudflared.service /etc/senvakara/cloudflared.env
systemctl daemon-reload
apt-get -y -q "${lock[@]}" purge cloudflared
rm -f /etc/apt/sources.list.d/cloudflared.sources /etc/apt/sources.list.d/cloudflared.list /usr/share/keyrings/cloudflare-main.gpg
apt-get -q "${lock[@]}" update || true
echo "cloudflared was removed together with the stored tunnel token. The tunnel still exists in your Cloudflare account."
