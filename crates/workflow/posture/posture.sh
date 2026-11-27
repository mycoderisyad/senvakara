# Senvakara host security posture (plan WF §5). Read-only, runs as the
# signed-in user without sudo, so it can run whenever the user asks without a
# password prompt. Prints one line per control:
#   SV_POSTURE<TAB><control><TAB>pass|fail|unknown|info<TAB><evidence>
# A value that needs root to read is reported as unknown, never guessed.
# The controls follow common Linux server hardening practice; this is not a
# CIS benchmark and claims no compliance.
set -u
export LC_ALL=C

out() {
  printf 'SV_POSTURE\t%s\t%s\t%s\n' "$1" "$2" \
    "$(printf '%s' "$3" | tr '\t\r\n' '   ' | cut -c1-200)"
}
have() { command -v "$1" >/dev/null 2>&1; }

os_id=unknown
os_version=unknown
if [ -r /etc/os-release ]; then
  os_id=$(. /etc/os-release && printf '%s' "${ID:-unknown}")
  os_version=$(. /etc/os-release && printf '%s' "${VERSION_ID:-unknown}")
fi
printf 'SV_POSTURE_OS\t%s\t%s\n' "$os_id" "$os_version"

# ---- sshd: its global settings as sshd reads them. The first value of a
# keyword wins, Include is expanded where it stands, and the global section
# ends at the first Match.
sshd_lines() {
  local file=$1 depth=$2 line kw rest pattern inc
  [ "$depth" -gt 4 ] && return 0
  if [ ! -r "$file" ]; then
    echo "__unreadable__ $file"
    return 0
  fi
  while IFS= read -r line || [ -n "$line" ]; do
    line=$(printf '%s' "$line" | sed 's/^[[:space:]]*\([A-Za-z]*\)[[:space:]]*=[[:space:]]*/\1 /')
    read -r kw rest <<<"$line"
    [ -z "$kw" ] && continue
    case $kw in \#*) continue ;; esac
    kw=$(printf '%s' "$kw" | tr 'A-Z' 'a-z')
    case $kw in
      match)
        echo "__match__"
        return 1
        ;;
      include)
        for pattern in $rest; do
          case $pattern in /*) ;; *) pattern="/etc/ssh/$pattern" ;; esac
          for inc in $pattern; do
            [ -e "$inc" ] || continue
            sshd_lines "$inc" $((depth + 1)) || return 1
          done
        done
        ;;
      *) printf '%s %s\n' "$kw" "$rest" ;;
    esac
  done <"$file"
}

if [ -e /etc/ssh/sshd_config ]; then
  sshd_cfg=$(sshd_lines /etc/ssh/sshd_config 0)
  first() { printf '%s\n' "$sshd_cfg" | awk -v k="$1" '$1 == k { print $2; exit }'; }
  unreadable=$(printf '%s\n' "$sshd_cfg" | awk '$1 == "__unreadable__" { print $2 }' | head -n 3 | tr '\n' ' ')

  root=$(first permitrootlogin)
  if [ -n "$root" ]; then
    case $root in
      no | prohibit-password | without-password | forced-commands-only)
        out root_login pass "PermitRootLogin $root" ;;
      *) out root_login fail "PermitRootLogin $root" ;;
    esac
  elif [ -n "$unreadable" ]; then
    out root_login unknown "cannot read $unreadable"
  else
    out root_login pass "PermitRootLogin not set (OpenSSH default: prohibit-password)"
  fi

  pw=$(first passwordauthentication)
  kbd=$(first kbdinteractiveauthentication)
  [ -z "$kbd" ] && kbd=$(first challengeresponseauthentication)
  if [ -n "$unreadable" ] && { [ -z "$pw" ] || [ -z "$kbd" ]; }; then
    out password_auth unknown "cannot read $unreadable"
  else
    evidence="PasswordAuthentication ${pw:-yes (default)}, KbdInteractiveAuthentication ${kbd:-yes (default)}"
    if [ "$pw" = no ] && [ "$kbd" = no ]; then
      out password_auth pass "$evidence"
    else
      out password_auth fail "$evidence"
    fi
  fi
else
  out root_login unknown "no /etc/ssh/sshd_config"
  out password_auth unknown "no /etc/ssh/sshd_config"
fi

# ---- Firewall. Reading live rules needs root; the ufw settings files and
# the service state do not.
if [ -r /etc/ufw/ufw.conf ]; then
  enabled=$(sed -n 's/^ENABLED=//p' /etc/ufw/ufw.conf | tr -d '"' | tail -n 1)
  policy=$(sed -n 's/^DEFAULT_INPUT_POLICY=//p' /etc/default/ufw 2>/dev/null | tr -d '"' | tail -n 1)
  service=$(systemctl is-active ufw 2>/dev/null || true)
  evidence="ufw enabled=${enabled:-?}, service ${service:-unknown}, incoming ${policy:-?}"
  if [ "$enabled" = yes ] && [ "$service" = active ] && { [ "$policy" = DROP ] || [ "$policy" = REJECT ]; }; then
    out firewall pass "$evidence"
  else
    out firewall fail "$evidence"
  fi
elif [ "$(systemctl is-active nftables 2>/dev/null)" = active ]; then
  out firewall unknown "nftables is active; its rules need root to read"
elif [ "$(systemctl is-active firewalld 2>/dev/null)" = active ]; then
  out firewall unknown "firewalld is active; its rules need root to read"
else
  out firewall fail "no ufw, nftables or firewalld service is active"
fi

# ---- Automatic security updates.
installed() { dpkg-query -W -f='${Status}' "$1" 2>/dev/null | grep -q 'install ok installed'; }
if have apt-config; then
  uu=$(apt-config dump APT::Periodic::Unattended-Upgrade 2>/dev/null | sed -n 's/^APT::Periodic::Unattended-Upgrade "\(.*\)";$/\1/p' | tail -n 1)
  lists=$(apt-config dump APT::Periodic::Update-Package-Lists 2>/dev/null | sed -n 's/^APT::Periodic::Update-Package-Lists "\(.*\)";$/\1/p' | tail -n 1)
  if ! installed unattended-upgrades; then
    out auto_updates fail "unattended-upgrades is not installed"
  elif [ -n "$uu" ] && [ "$uu" != 0 ] && [ -n "$lists" ] && [ "$lists" != 0 ]; then
    out auto_updates pass "unattended-upgrades installed, Unattended-Upgrade \"$uu\", Update-Package-Lists \"$lists\""
  else
    out auto_updates fail "unattended-upgrades installed but not switched on (Unattended-Upgrade \"${uu:-unset}\", Update-Package-Lists \"${lists:-unset}\")"
  fi
else
  out auto_updates unknown "apt is not available"
fi

# ---- fail2ban.
if installed fail2ban; then
  state=$(systemctl is-active fail2ban 2>/dev/null || true)
  if [ "$state" = active ]; then
    out fail2ban pass "fail2ban service active"
  else
    out fail2ban fail "fail2ban installed, service ${state:-unknown}"
  fi
else
  out fail2ban fail "fail2ban is not installed"
fi

# ---- Pending security updates, from the package lists already on the host
# (they are not refreshed here: that needs root and the network).
if have apt-get; then
  newest=$(ls -t /var/lib/apt/lists/*_Packages 2>/dev/null | head -n 1)
  when=unknown
  [ -n "$newest" ] && when=$(date -r "$newest" +%Y-%m-%d 2>/dev/null || echo unknown)
  if sim=$(apt-get -s -q dist-upgrade 2>/dev/null); then
    total=$(printf '%s\n' "$sim" | grep -c '^Inst ' || true)
    security=$(printf '%s\n' "$sim" | grep '^Inst ' | grep -ci 'security' || true)
    if [ "$security" -eq 0 ]; then
      out security_updates pass "0 security updates pending ($total in all; package lists from $when)"
    else
      out security_updates fail "$security security updates pending ($total in all; package lists from $when)"
    fi
  else
    out security_updates unknown "apt-get could not simulate an upgrade"
  fi
else
  out security_updates unknown "apt is not available"
fi

# ---- Reboot required.
running=$(uname -r)
if [ -e /run/reboot-required ] || [ -e /var/run/reboot-required ]; then
  pkgs=$(cat /run/reboot-required.pkgs 2>/dev/null | sort -u | head -n 5 | tr '\n' ' ')
  out reboot_required fail "the host asks for a reboot${pkgs:+ (for $pkgs)}"
else
  newest=$(ls /boot/vmlinuz-* 2>/dev/null | sed 's|^/boot/vmlinuz-||' | sort -V | tail -n 1)
  if [ -n "$newest" ] && [ "$newest" != "$running" ]; then
    out reboot_required fail "running kernel $running, newest installed $newest"
  else
    out reboot_required pass "no reboot flag; running kernel $running"
  fi
fi

# ---- A non-root user who can use sudo.
admins=""
for group in sudo admin wheel; do
  members=$(getent group "$group" 2>/dev/null | cut -d: -f4)
  [ -n "$members" ] && admins="$admins,$members"
done
admins=$(printf '%s' "$admins" | tr ',' '\n' | grep -v -e '^$' -e '^root$' | sort -u | tr '\n' ' ')
if [ -n "$admins" ]; then
  out sudo_user pass "in a sudo group: $admins"
else
  out sudo_user fail "no user besides root is in the sudo group"
fi

# ---- Time synchronisation.
if have timedatectl && synced=$(timedatectl show -p NTPSynchronized --value 2>/dev/null); then
  ntp=$(timedatectl show -p NTP --value 2>/dev/null || true)
  zone=$(timedatectl show -p Timezone --value 2>/dev/null || true)
  if [ "$synced" = yes ]; then
    out time_sync pass "clock synchronised, NTP ${ntp:-?}, time zone ${zone:-?}"
  else
    out time_sync fail "clock not synchronised, NTP ${ntp:-?}, time zone ${zone:-?}"
  fi
else
  out time_sync unknown "timedatectl did not answer (no systemd?)"
fi

# ---- Listening TCP ports (information: which of them should be open is the
# user's call).
if have ss; then
  ports=$(ss -Hltn 2>/dev/null | awk '{ print $4 }')
  public=$(printf '%s\n' "$ports" | grep -v -e '^127\.' -e '^\[::1\]' -e '^$' | sed 's/.*://' | sort -un | tr '\n' ' ')
  local_only=$(printf '%s\n' "$ports" | grep -e '^127\.' -e '^\[::1\]' | sed 's/.*://' | sort -un | tr '\n' ' ')
  out listening_ports info "reachable from outside: ${public:-none}; this host only: ${local_only:-none}"
else
  out listening_ports unknown "ss is not available"
fi
