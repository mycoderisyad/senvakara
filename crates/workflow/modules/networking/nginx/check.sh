# Read-only. Up to date when nginx runs from the nginx.org repository on the chosen
# channel and the Senvakara proxy site matches the settings (or is absent without a host name).
export LC_ALL=C
. /etc/os-release
channel="${SV_CHANNEL:-stable}"
domain="${SV_DOMAIN:-}"
upstream="${SV_UPSTREAM:-}"
conf=/etc/nginx/conf.d/senvakara-proxy.conf
mapf=/etc/nginx/conf.d/senvakara-upgrade-map.conf
list=/etc/apt/sources.list.d/nginx.sources
key=/usr/share/keyrings/nginx-archive-keyring.gpg

path=packages
[ "$channel" = mainline ] && path=packages/mainline
want_repo=$(printf 'Types: deb\nURIs: https://nginx.org/%s/%s\nSuites: %s\nComponents: nginx\nSigned-By: %s\n' "$path" "$ID" "$VERSION_CODENAME" "$key")

render_site() {
  printf '# Written by Senvakara (nginx node). Change it there.\n'
  printf 'server {\n\tlisten 80;\n\tserver_name %s;\n\tlocation / {\n' "$domain"
  printf '\t\tproxy_pass http://%s;\n' "$upstream"
  printf '\t\tproxy_set_header Host $host;\n\t\tproxy_set_header X-Real-IP $remote_addr;\n'
  printf '\t\tproxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n'
  printf '\t\tproxy_set_header X-Forwarded-Proto $scheme;\n\t\tproxy_http_version 1.1;\n'
  printf '\t\tproxy_set_header Upgrade $http_upgrade;\n\t\tproxy_set_header Connection $connection_upgrade;\n'
  printf '\t}\n}\n'
}
render_map() {
  printf '# Written by Senvakara (nginx node). Change it there.\n'
  printf 'map $http_upgrade $connection_upgrade {\n\tdefault upgrade;\n\t"" close;\n}\n'
}

if ! dpkg-query -W -f='${Status}' nginx 2>/dev/null | grep -q 'install ok installed'; then
  echo "nginx is not installed."; exit 10
fi
if [ ! -s "$key" ] || [ "$(cat "$list" 2>/dev/null)" != "$want_repo" ]; then
  echo "The nginx.org apt repository is not set up for the $channel channel."; exit 10
fi
if ! systemctl is-active --quiet nginx; then
  echo "nginx is not running."; exit 10
fi
if [ -n "$domain" ]; then
  if ! [[ "$upstream" =~ ^[A-Za-z0-9.-]+:[0-9]{1,5}$ ]]; then
    echo "The proxy target must look like 127.0.0.1:3001."; exit 2
  fi
  if [ "$(cat "$conf" 2>/dev/null)" != "$(render_site)" ] || [ "$(cat "$mapf" 2>/dev/null)" != "$(render_map)" ]; then
    echo "The proxy site for $domain is not written yet."; exit 10
  fi
elif [ -e "$conf" ] || [ -e "$mapf" ]; then
  echo "A Senvakara proxy site exists but no host name is set."; exit 10
fi
echo "nginx ($channel) is installed and running."
