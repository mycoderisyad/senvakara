# Add Cloudflare's apt repository, install cloudflared, store the tunnel token in a root-only
# file and run `cloudflared tunnel run` from a Senvakara systemd unit that reads it as
# TUNNEL_TOKEN (so the token never appears in argv).
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
token="${SV_TOKEN:-}"
lock=(-o DPkg::Lock::Timeout=300)
key=/usr/share/keyrings/cloudflare-main.gpg
list=/etc/apt/sources.list.d/cloudflared.sources
envdir=/etc/senvakara
envfile=$envdir/cloudflared.env
unit=/etc/systemd/system/senvakara-cloudflared.service

if [ -z "$token" ]; then
  echo "No tunnel token was given."; exit 2
fi
if ! [[ "$token" =~ ^[A-Za-z0-9+/=_.-]+$ ]]; then
  echo "The tunnel token has characters a token never contains."; exit 2
fi

render_unit() {
  printf '# Written by Senvakara (Cloudflare Tunnel node). Change it there.\n'
  printf '[Unit]\nDescription=Cloudflare Tunnel (Senvakara)\nAfter=network-online.target\nWants=network-online.target\n\n'
  printf '[Service]\nType=simple\nEnvironmentFile=%s\n' "$envfile"
  printf 'ExecStart=/usr/bin/cloudflared --no-autoupdate tunnel run\n'
  printf 'Restart=on-failure\nRestartSec=5\nTimeoutStartSec=0\n\n[Install]\nWantedBy=multi-user.target\n'
}

apt-get -q "${lock[@]}" update
apt-get -y -q "${lock[@]}" install curl gnupg ca-certificates

if [ ! -s "$key" ]; then
  mkdir -p --mode=0755 /usr/share/keyrings
  tmp=$(mktemp)
  curl -fsSL https://pkg.cloudflare.com/cloudflare-main.gpg >"$tmp"
  install -m 0644 "$tmp" "$key"
  rm -f "$tmp"
fi

tmp=$(mktemp)
printf 'Types: deb\nURIs: https://pkg.cloudflare.com/cloudflared\nSuites: any\nComponents: main\nSigned-By: %s\n' "$key" >"$tmp"
if ! cmp -s "$tmp" "$list"; then
  install -m 0644 "$tmp" "$list"
fi
rm -f "$tmp"
rm -f /etc/apt/sources.list.d/cloudflared.list

apt-get -q "${lock[@]}" update
apt-get -y -q "${lock[@]}" install cloudflared

# Token file: created with umask 077, root only. printf is a builtin, so no argv exposure.
install -d -m 0755 "$envdir"
tmp=$(umask 077; mktemp)
printf 'TUNNEL_TOKEN=%s\n' "$token" >"$tmp"
tokenchanged=0
if ! cmp -s "$tmp" "$envfile"; then
  install -m 0600 -o root -g root "$tmp" "$envfile"
  tokenchanged=1
else
  chmod 0600 "$envfile"
fi
rm -f "$tmp"

tmp=$(mktemp)
render_unit >"$tmp"
changed=0
if ! cmp -s "$tmp" "$unit"; then
  install -m 0644 "$tmp" "$unit"
  changed=1
fi
rm -f "$tmp"
[ "$changed" = 1 ] && systemctl daemon-reload

systemctl enable senvakara-cloudflared
# restart picks up a changed token file; a no-op change leaves the tunnel running.
if [ "$changed" = 1 ] || [ "$tokenchanged" = 1 ] || ! systemctl is-active --quiet senvakara-cloudflared; then
  systemctl restart senvakara-cloudflared
fi
sleep 2
systemctl is-active senvakara-cloudflared
