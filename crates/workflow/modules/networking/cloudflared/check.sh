# Read-only. Up to date when cloudflared is installed from Cloudflare's repository and the
# Senvakara tunnel unit runs with the token file holding the given token. Never prints the token.
export LC_ALL=C
token="${SV_TOKEN:-}"
list=/etc/apt/sources.list.d/cloudflared.sources
key=/usr/share/keyrings/cloudflare-main.gpg
envfile=/etc/senvakara/cloudflared.env
unit=/etc/systemd/system/senvakara-cloudflared.service

render_unit() {
  printf '# Written by Senvakara (Cloudflare Tunnel node). Change it there.\n'
  printf '[Unit]\nDescription=Cloudflare Tunnel (Senvakara)\nAfter=network-online.target\nWants=network-online.target\n\n'
  printf '[Service]\nType=simple\nEnvironmentFile=%s\n' "$envfile"
  printf 'ExecStart=/usr/bin/cloudflared --no-autoupdate tunnel run\n'
  printf 'Restart=on-failure\nRestartSec=5\nTimeoutStartSec=0\n\n[Install]\nWantedBy=multi-user.target\n'
}

if [ -z "$token" ]; then
  echo "No tunnel token was given."; exit 2
fi
if ! dpkg-query -W -f='${Status}' cloudflared 2>/dev/null | grep -q 'install ok installed'; then
  echo "cloudflared is not installed."; exit 10
fi
want_list=$(printf 'Types: deb\nURIs: https://pkg.cloudflare.com/cloudflared\nSuites: any\nComponents: main\nSigned-By: %s\n' "$key")
if [ ! -s "$key" ] || [ "$(cat "$list" 2>/dev/null)" != "$want_list" ]; then
  echo "The Cloudflare apt repository is not set up."; exit 10
fi
if [ "$(cat "$unit" 2>/dev/null)" != "$(render_unit)" ]; then
  echo "The Senvakara tunnel service is not written yet."; exit 10
fi
if [ "$(stat -c '%a %U' "$envfile" 2>/dev/null)" != "600 root" ]; then
  echo "The token file is missing or has the wrong mode."; exit 10
fi
if [ "$(cat "$envfile")" != "TUNNEL_TOKEN=$token" ]; then
  echo "The token file holds a different token."; exit 10
fi
if ! systemctl is-enabled --quiet senvakara-cloudflared || ! systemctl is-active --quiet senvakara-cloudflared; then
  echo "The tunnel service is not enabled and running."; exit 10
fi
echo "The Cloudflare tunnel service is installed and running."
