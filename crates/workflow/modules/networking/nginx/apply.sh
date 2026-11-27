# Add nginx.org's apt repository for the chosen channel, install nginx, optionally write a
# plain-HTTP reverse-proxy site, test the config and reload.
export DEBIAN_FRONTEND=noninteractive LC_ALL=C
. /etc/os-release
channel="${SV_CHANNEL:-stable}"
domain="${SV_DOMAIN:-}"
upstream="${SV_UPSTREAM:-}"
lock=(-o DPkg::Lock::Timeout=300)
key=/usr/share/keyrings/nginx-archive-keyring.gpg
list=/etc/apt/sources.list.d/nginx.sources
conf=/etc/nginx/conf.d/senvakara-proxy.conf
mapf=/etc/nginx/conf.d/senvakara-upgrade-map.conf

case "$channel" in stable | mainline) ;; *) echo "The channel must be stable or mainline."; exit 2 ;; esac
case "$ID" in debian | ubuntu) ;; *) echo "nginx.org packages are used on Debian and Ubuntu only."; exit 2 ;; esac
if [ -n "$domain" ]; then
  if ! [[ "$upstream" =~ ^[A-Za-z0-9.-]+:[0-9]{1,5}$ ]]; then
    echo "The proxy target must look like 127.0.0.1:3001."; exit 2
  fi
  if ! [[ "$domain" =~ ^([A-Za-z0-9]([A-Za-z0-9-]{0,61}[A-Za-z0-9])?\.)+[A-Za-z]{2,63}$ ]]; then
    echo "The host name is not a valid name."; exit 2
  fi
fi

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

apt-get -q "${lock[@]}" update
keyring=debian-archive-keyring
[ "$ID" = ubuntu ] && keyring=ubuntu-keyring
apt-get -y -q "${lock[@]}" install curl gnupg2 ca-certificates lsb-release "$keyring"

if [ ! -s "$key" ]; then
  tmp=$(mktemp)
  curl -fsSL https://nginx.org/keys/nginx_signing.key | gpg --dearmor >"$tmp"
  install -m 0644 "$tmp" "$key"
  rm -f "$tmp"
fi

path=packages
[ "$channel" = mainline ] && path=packages/mainline
tmp=$(mktemp)
printf 'Types: deb\nURIs: https://nginx.org/%s/%s\nSuites: %s\nComponents: nginx\nSigned-By: %s\n' "$path" "$ID" "$VERSION_CODENAME" "$key" >"$tmp"
if ! cmp -s "$tmp" "$list"; then
  install -m 0644 "$tmp" "$list"
fi
rm -f "$tmp"
rm -f /etc/apt/sources.list.d/nginx.list

# Prefer nginx.org packages over the distribution's, as the vendor page says.
pin=/etc/apt/preferences.d/99nginx
tmp=$(mktemp)
printf 'Package: *\nPin: origin nginx.org\nPin: release o=nginx\nPin-Priority: 900\n' >"$tmp"
if ! cmp -s "$tmp" "$pin"; then
  install -m 0644 "$tmp" "$pin"
fi
rm -f "$tmp"

apt-get -q "${lock[@]}" update
apt-get -y -q "${lock[@]}" install nginx

changed=0
if [ -n "$domain" ]; then
  tmp=$(mktemp)
  render_map >"$tmp"
  if ! cmp -s "$tmp" "$mapf"; then install -m 0644 "$tmp" "$mapf"; changed=1; fi
  render_site >"$tmp"
  if ! cmp -s "$tmp" "$conf"; then install -m 0644 "$tmp" "$conf"; changed=1; fi
  rm -f "$tmp"
elif [ -e "$conf" ] || [ -e "$mapf" ]; then
  rm -f "$conf" "$mapf"
  changed=1
fi

nginx -t
systemctl enable nginx
# Reload only after a change, so a second apply leaves nginx alone.
if ! systemctl is-active --quiet nginx; then
  systemctl start nginx
elif [ "$changed" = 1 ]; then
  systemctl reload nginx
fi
systemctl is-active nginx
