#!/bin/sh
# Install first, then persist PATH for the user's shell. No root privileges needed.
set -eu

cli_only=false
path_only=false
case "${1-}" in
    '') ;;
    --cli-only) cli_only=true; shift ;;
    --path-only) path_only=true; shift ;;
    *) echo 'Usage: ./install.sh [--cli-only | --path-only]' >&2; exit 2 ;;
esac
if [ "$#" -ne 0 ]; then
    echo 'Usage: ./install.sh [--cli-only | --path-only]' >&2
    exit 2
fi

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
install_root=${CARGO_INSTALL_ROOT:-${CARGO_HOME:-"$HOME/.cargo"}}
case "$install_root" in
    /*) ;;
    *) install_root="$(pwd)/$install_root" ;;
esac
if [ "$path_only" = true ]; then
    if [ ! -x "$install_root/bin/codesync" ]; then
        echo 'Codesync is not installed in the selected Cargo root. Run ./install.sh first.' >&2
        exit 1
    fi
else
cargo_bin=$(command -v cargo || true)
if [ -z "$cargo_bin" ] && [ -x "${CARGO_HOME:-$HOME/.cargo}/bin/cargo" ]; then
    cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin/cargo"
fi
if [ -z "$cargo_bin" ]; then
    echo 'Rust/Cargo is required. Install Rust, then run this installer again.' >&2
    exit 1
fi
if [ "$cli_only" = true ]; then
    "$cargo_bin" install --path "$project_dir" --root "$install_root" --force --no-default-features --bin codesync
else
    "$cargo_bin" install --path "$project_dir" --root "$install_root" --force
fi

fi

# Quote literal paths, including spaces, apostrophes, and shell metacharacters.
quote_sh() {
    printf "'"
    printf '%s' "$1" | sed "s/'/'\\\\''/g"
    printf "'"
}
bin_dir=$install_root/bin
quoted_bin=$(quote_sh "$bin_dir")
path_line="case \":\$PATH:\" in *:$quoted_bin:*) ;; *) export PATH=$quoted_bin:\"\$PATH\" ;; esac"
append_path() {
    file=$1
    mkdir -p -- "$(dirname -- "$file")"
    if [ ! -f "$file" ] || ! grep -Fqx -- "$path_line" "$file"; then
        printf '\n# Codesync: make installed commands available in every directory.\n%s\n' "$path_line" >> "$file"
    fi
}
login_shell=${SHELL:-sh}
case "${login_shell##*/}" in
    fish)
        fish_dir=${XDG_CONFIG_HOME:-"$HOME/.config"}/fish/conf.d
        mkdir -p -- "$fish_dir"
        # Fish single quotes escape backslashes and apostrophes directly.
        fish_bin=$(printf '%s' "$bin_dir" | sed "s/\\\\/\\\\\\\\/g; s/'/\\\\'/g")
        fish_line="contains -- '$fish_bin' \$PATH; or set -gx PATH '$fish_bin' \$PATH"
        fish_file=$fish_dir/codesync-path.fish
        if [ ! -f "$fish_file" ] || ! grep -Fqx -- "$fish_line" "$fish_file"; then
            printf '\n# Codesync PATH\n%s\n' "$fish_line" >> "$fish_file"
        fi
        ;;
    zsh)
        append_path "${ZDOTDIR:-$HOME}/.zshrc"
        append_path "${ZDOTDIR:-$HOME}/.zprofile"
        ;;
    *)
        append_path "$HOME/.bashrc"
        if [ -f "$HOME/.bash_profile" ]; then
            append_path "$HOME/.bash_profile"
        elif [ -f "$HOME/.bash_login" ]; then
            append_path "$HOME/.bash_login"
        else
            append_path "$HOME/.profile"
        fi
        ;;
esac
# Reuse a user-owned bin directory already present in the calling shell's PATH.
# This makes the command visible immediately, without trying to modify the parent shell.
available_now=false
case ":$PATH:" in *:"$bin_dir":*) available_now=true ;; esac
for user_bin in "$HOME/.local/bin" "$HOME/bin"; do
    [ "$available_now" = false ] || break
    case ":$PATH:" in *:"$user_bin":*) ;; *) continue ;; esac
    [ -d "$user_bin" ] && [ -w "$user_bin" ] || continue
    for binary in codesync codesync-gui; do
        [ -x "$bin_dir/$binary" ] || continue
        if [ ! -e "$user_bin/$binary" ] && [ ! -L "$user_bin/$binary" ]; then
            ln -s -- "$bin_dir/$binary" "$user_bin/$binary"
        fi
    done
    if [ "$user_bin/codesync" -ef "$bin_dir/codesync" ]; then
        available_now=true
    fi
done
printf '\nCodesync PATH setup completed.\n'
if [ "$available_now" = true ]; then
    printf 'codesync is available on the PATH passed to this installer.\n'
else
    printf 'Open a new terminal, or run this once in the current terminal:\n'
    case "${login_shell##*/}" in
        fish) printf '  %s\n' "$fish_line" ;;
        *) printf '  export PATH=%s:"$PATH"\n' "$quoted_bin" ;;
    esac
fi
if [ "$cli_only" = true ]; then
    printf 'CLI-only installation: run codesync from your project directory.\n'
else
    printf 'Launch: codesync gui\n'
    printf 'Direct launch (works without PATH): %s/codesync gui\n' "$quoted_bin"
fi
