set -eu
# Root only; local authorization is handled by pkexec, remote by sudo.
# Close the password input before running installers.
exec </dev/null
if ! command -v tailscale >/dev/null 2>&1; then
    printf '%s\n' 'Installing Tailscale...'
    if command -v pacman >/dev/null 2>&1; then
        pacman -S --needed --noconfirm tailscale
    elif command -v apk >/dev/null 2>&1; then
        apk add tailscale
    else
        installer=$(mktemp)
        trap 'rm -f "$installer"' EXIT HUP INT TERM
        if command -v curl >/dev/null 2>&1; then
            curl --proto '=https' --tlsv1.2 -fsSL --connect-timeout 20 --max-time 120 https://tailscale.com/install.sh -o "$installer"
        elif command -v wget >/dev/null 2>&1; then
            wget -q -T 120 -O "$installer" https://tailscale.com/install.sh
        else
            printf '%s\n' 'Install curl or wget, then retry Tailscale setup.' >&2
            exit 1
        fi
        sh "$installer"
        rm -f "$installer"
        trap - EXIT HUP INT TERM
    fi
fi
if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    systemctl enable --now tailscaled
elif command -v rc-service >/dev/null 2>&1; then
    rc-update add tailscale default
    rc-service tailscale start
else
    printf '%s\n' 'No supported service manager. Start tailscaled manually, then retry.' >&2
    exit 1
fi
if tailscale status --json 2>/dev/null | grep -q '"BackendState":[[:space:]]*"Running"'; then
    printf '%s\n' 'Tailscale is already connected.'
else
    printf '%s\n' 'Waiting for Tailscale sign-in. Use the sign-in button in Codesync.'
    tailscale up --timeout=120s
fi
