set -eu
exec </dev/null
if ! command -v sshd >/dev/null 2>&1 || ! command -v rsync >/dev/null 2>&1 || ! command -v sha256sum >/dev/null 2>&1; then
    if command -v pacman >/dev/null 2>&1; then
        pacman -S --needed --noconfirm openssh rsync coreutils
    elif command -v apk >/dev/null 2>&1; then
        apk add openssh rsync coreutils
    elif command -v apt-get >/dev/null 2>&1; then
        apt-get update
        apt-get install -y openssh-server openssh-client rsync coreutils
    elif command -v dnf >/dev/null 2>&1; then
        dnf install -y openssh-server openssh-clients rsync coreutils
    else
        echo 'Install OpenSSH server, rsync, and sha256sum on this host, then retry.' >&2
        exit 1
    fi
fi
# Generates missing host keys only. Existing identities and SSH settings stay intact.
ssh-keygen -A
if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    if systemctl cat sshd.service >/dev/null 2>&1; then
        systemctl enable --now sshd.service
    else
        systemctl enable --now ssh.service
    fi
elif command -v rc-service >/dev/null 2>&1; then
    rc-update add sshd default
    rc-service sshd start
else
    echo 'Start the OpenSSH server manually with your service manager.' >&2
    exit 1
fi
sshd -t
printf '%s\n' 'This host is ready for SSH connections. Add its reachable address, SSH port, and account on the other host. If a firewall blocks SSH, allow access from your trusted network.'
