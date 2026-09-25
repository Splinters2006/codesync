set -eu
mode=$1
case "$mode" in status|disable) ;; *) exit 2 ;; esac
if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    systemctl list-units --no-legend >/dev/null
    if [ "$mode" = status ]; then
        if systemctl is-active --quiet ssh.service sshd.service ssh.socket sshd.socket; then
            printf '%s\n' enabled
        else
            printf '%s\n' disabled
        fi
    else
        # Stop socket activation as well as the service, so a connection cannot restart SSH.
        seen=' '
        for unit in ssh.socket sshd.socket ssh.service sshd.service; do
            if systemctl cat "$unit" >/dev/null 2>&1; then
                canonical=$(systemctl show --property=Id --value "$unit")
                case "$seen" in *" $canonical "*) continue ;; esac
                seen="$seen$canonical "
                systemctl disable --now "$canonical"
            fi
        done
        if systemctl is-active --quiet ssh.service sshd.service ssh.socket sshd.socket; then
            echo 'SSH is still active. Check the service manager.' >&2
            exit 1
        fi
        printf '%s\n' 'Incoming SSH stopped and disabled at startup. Existing sessions may remain open.'
    fi
elif command -v rc-service >/dev/null 2>&1; then
    if [ "$mode" = status ]; then
        if rc-service sshd status >/dev/null 2>&1; then printf '%s\n' enabled; else printf '%s\n' disabled; fi
    else
        # Remove SSH from every runlevel where it is enabled.
        for runlevel in /etc/runlevels/*; do
            if [ -e "$runlevel/sshd" ]; then rc-update del sshd "${runlevel##*/}"; fi
        done
        if rc-service sshd status >/dev/null 2>&1; then rc-service sshd stop; fi
        if rc-service sshd status >/dev/null 2>&1; then
            echo 'SSH is still active. Check the service manager.' >&2
            exit 1
        fi
        printf '%s\n' 'Incoming SSH stopped and disabled at startup. Existing sessions may remain open.'
    fi
else
    echo 'Cannot manage SSH: this host needs systemd or OpenRC.' >&2
    exit 1
fi
