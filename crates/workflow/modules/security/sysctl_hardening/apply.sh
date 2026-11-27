# Write the drop-in (read again at every boot) and load it now. -e skips
# keys this kernel does not have instead of failing.
export LC_ALL=C
conf=/etc/sysctl.d/90-senvakara-network.conf
tmp=$(mktemp)
cat >"$tmp" <<'EOF'
# Written by Senvakara (network kernel settings node).
net.ipv4.tcp_syncookies = 1
net.ipv4.conf.all.accept_redirects = 0
net.ipv4.conf.default.accept_redirects = 0
net.ipv6.conf.all.accept_redirects = 0
net.ipv6.conf.default.accept_redirects = 0
net.ipv4.conf.all.secure_redirects = 0
net.ipv4.conf.default.secure_redirects = 0
net.ipv4.conf.all.send_redirects = 0
net.ipv4.conf.default.send_redirects = 0
net.ipv4.conf.all.accept_source_route = 0
net.ipv4.conf.default.accept_source_route = 0
net.ipv6.conf.all.accept_source_route = 0
net.ipv6.conf.default.accept_source_route = 0
net.ipv4.icmp_echo_ignore_broadcasts = 1
net.ipv4.icmp_ignore_bogus_error_responses = 1
EOF
install -m 0644 "$tmp" "$conf"
rm -f "$tmp"
sysctl -e -p "$conf"
