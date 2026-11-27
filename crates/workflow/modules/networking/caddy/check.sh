# Read-only. Up to date when Caddy is installed from the official repository, running, and
# the Senvakara proxy site matches the settings (or is absent when no domain is set).
export LC_ALL=C
. /etc/os-release
domain="${SV_DOMAIN:-}"
upstream="${SV_UPSTREAM:-}"
site=/etc/caddy/conf.d/senvakara-proxy.caddy
list=/etc/apt/sources.list.d/caddy-stable.sources
key=/usr/share/keyrings/caddy-stable-archive-keyring.gpg

if ! dpkg-query -W -f='${Status}' caddy 2>/dev/null | grep -q 'install ok installed'; then
  echo "Caddy is not installed."; exit 10
fi
want_list=$(printf 'Types: deb\nURIs: https://dl.cloudsmith.io/public/caddy/stable/deb/debian\nSuites: any-version\nComponents: main\nSigned-By: %s\n' "$key")
if [ ! -s "$key" ] || [ "$(cat "$list" 2>/dev/null)" != "$want_list" ]; then
  echo "The Caddy apt repository is not set up."; exit 10
fi
if ! systemctl is-active --quiet caddy; then
  echo "Caddy is not running."; exit 10
fi
if [ -n "$domain" ]; then
  if ! [[ "$upstream" =~ ^[A-Za-z0-9.-]+:[0-9]{1,5}$ ]]; then
    echo "The proxy target must look like 127.0.0.1:3001."; exit 2
  fi
  want=$(printf '# Written by Senvakara (Caddy node). Change it there.\n%s {\n\treverse_proxy %s\n}\n' "$domain" "$upstream")
  if [ "$(cat "$site" 2>/dev/null)" != "$want" ]; then
    echo "The proxy site for $domain is not written yet."; exit 10
  fi
  if ! grep -qs '^import /etc/caddy/conf.d/\*.caddy$' /etc/caddy/Caddyfile; then
    echo "/etc/caddy/Caddyfile does not import the Senvakara site folder."; exit 10
  fi
elif [ -e "$site" ]; then
  echo "A Senvakara proxy site exists but no domain is set."; exit 10
fi
echo "Caddy is installed and running."
