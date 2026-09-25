set -eu

if command -v rsync >/dev/null 2>&1; then
    rsync --version
    printf '%s\n' 'Server is ready: rsync is already installed.'
    exit 0
fi

# Select only known package managers; never execute downloaded installer scripts.
if command -v apk >/dev/null 2>&1; then
    manager=apk
elif command -v apt-get >/dev/null 2>&1; then
    manager=apt-get
elif command -v dnf >/dev/null 2>&1; then
    manager=dnf
elif command -v pacman >/dev/null 2>&1; then
    manager=pacman
else
    printf '%s\n' 'No supported package manager found. Install rsync on the server manually.' >&2
    exit 1
fi

as_root() {
    if [ "$(id -u)" = 0 ]; then
        "$@"
    elif command -v sudo >/dev/null 2>&1; then
        sudo -- "$@"
    elif command -v doas >/dev/null 2>&1; then
        doas "$@"
    else
        printf '%s\n' 'Installing rsync requires root, sudo, or doas on the server.' >&2
        exit 1
    fi
}

printf 'Installing rsync with %s. Administrator authentication may be required.\n' "$manager"
case "$manager" in
    apk) as_root apk add rsync ;;
    apt-get) as_root apt-get update; as_root apt-get install -y rsync ;;
    dnf) as_root dnf install -y rsync ;;
    pacman) as_root pacman -S --needed --noconfirm rsync ;;
esac

if ! command -v rsync >/dev/null 2>&1; then
    printf '%s\n' 'Installation finished but rsync is not on the remote PATH.' >&2
    exit 1
fi
rsync --version
printf '%s\n' 'Server is ready. You can now run codesync push.'
