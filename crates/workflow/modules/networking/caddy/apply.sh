# Add Caddy's official apt repository, install caddy, optionally write a reverse-proxy
# site for one domain, validate the config and reload.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
. /etc/os-release
domain="${SV_DOMAIN:-}"
upstream="${SV_UPSTREAM:-}"
lock=(-o DPkg::Lock::Timeout=300)
key=/usr/share/keyrings/caddy-stable-archive-keyring.gpg
list=/etc/apt/sources.list.d/caddy-stable.sources
site=/etc/caddy/conf.d/senvakara-proxy.caddy

if [ -n "$domain" ]; then
  if ! [[ "$upstream" =~ ^[A-Za-z0-9.-]+:[0-9]{1,5}$ ]]; then
    echo "The proxy target must look like 127.0.0.1:3001."; exit 2
  fi
  if ! [[ "$domain" =~ ^([A-Za-z0-9]([A-Za-z0-9-]{0,61}[A-Za-z0-9])?\.)+[A-Za-z]{2,63}$ ]]; then
    echo "The domain is not a valid host name."; exit 2
  fi
fi

apt-get -q "${lock[@]}" update
apt-get -y -q "${lock[@]}" install debian-keyring debian-archive-keyring apt-transport-https curl gnupg ca-certificates

if [ ! -s "$key" ]; then
  tmp=$(mktemp)
  curl -fsSL https://dl.cloudsmith.io/public/caddy/stable/gpg.key | gpg --dearmor >"$tmp"
  install -m 0644 "$tmp" "$key"
  rm -f "$tmp"
fi

tmp=$(mktemp)
cat >"$tmp" <<REPO
Types: deb
URIs: https://dl.cloudsmith.io/public/caddy/stable/deb/debian
Suites: any-version
Components: main
Signed-By: $key
REPO
if ! cmp -s "$tmp" "$list"; then
  install -m 0644 "$tmp" "$list"
fi
rm -f "$tmp"
# Drop a one-line .list file left by hand-following the vendor page: same repository twice.
rm -f /etc/apt/sources.list.d/caddy-stable.list

apt-get -q "${lock[@]}" update
apt-get -y -q "${lock[@]}" install caddy

changed=0
if [ -n "$domain" ]; then
  mkdir -p /etc/caddy/conf.d
  tmp=$(mktemp)
  printf '# Written by Senvakara (Caddy node). Change it there.\n%s {\n\treverse_proxy %s\n}\n' "$domain" "$upstream" >"$tmp"
  if ! cmp -s "$tmp" "$site"; then
    install -m 0644 "$tmp" "$site"
    changed=1
  fi
  rm -f "$tmp"
  if ! grep -qs '^import /etc/caddy/conf.d/\*.caddy$' /etc/caddy/Caddyfile; then
    printf '\nimport /etc/caddy/conf.d/*.caddy\n' >>/etc/caddy/Caddyfile
    changed=1
  fi
elif [ -e "$site" ]; then
  rm -f "$site"
  changed=1
fi

caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
systemctl enable caddy
# Reload only after a change, so a second apply leaves Caddy alone.
if ! systemctl is-active --quiet caddy; then
  systemctl start caddy
elif [ "$changed" = 1 ]; then
  systemctl reload caddy
fi
systemctl is-active caddy
