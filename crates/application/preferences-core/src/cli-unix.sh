set -eu

: "${HOME:?The host HOME directory is not set}"
case "$HOME" in /*) ;; *) echo 'The host HOME directory must be absolute' >&2; exit 1 ;; esac
bin="$HOME/.local/bin"
target="$bin/shrimply"

if [ "$1" = status ]; then
    if [ -e "$target" ] || [ -L "$target" ]; then
        printf '%s\n' "$target"
    else
        command -v shrimply || :
    fi
    exit 0
fi

[ "$1" = install ] && [ -n "$2" ] || exit 1
[ ! -d "$target" ] || { echo "$target is a directory; it cannot be replaced" >&2; exit 1; }
mkdir -p "$bin"

# Prepare the launcher in the same directory, then replace the directory entry.
# Never follow an existing symlink and overwrite its executable.
temporary=$(mktemp "$bin/.shrimply-cli.XXXXXX")
trap 'rm -f "$temporary"' EXIT HUP INT TERM
printf '#!/bin/sh\n# Shrimply CLI launcher\n%s\n' "$2" > "$temporary"
chmod 755 "$temporary"

# If Shrimply itself is already installed here, keep the actual application.
if [ -n "$3" ] && [ "$target" -ef "$3" ]; then
    rm -f "$temporary"
else
    mv -f "$temporary" "$target"
fi

# Put our command first, including when another Shrimply distribution is installed.
setup='\n# Shrimply CLI PATH\ncase "$PATH" in "$HOME/.local/bin"|"$HOME/.local/bin":*) ;; *) export PATH="$HOME/.local/bin:$PATH" ;; esac\n'
add_path() {
    if ! grep -Fq '# Shrimply CLI PATH' "$1" 2>/dev/null; then
        printf '%b' "$setup" >> "$1"
    fi
}

add_path "$HOME/.profile"
add_path "$HOME/.bashrc"
if [ -f "$HOME/.bash_profile" ]; then
    add_path "$HOME/.bash_profile"
elif [ -f "$HOME/.bash_login" ]; then
    add_path "$HOME/.bash_login"
fi
if command -v zsh >/dev/null 2>&1 || [ -e "${ZDOTDIR:-$HOME}/.zshrc" ]; then
    zsh_directory=${ZDOTDIR:-$HOME}
    if command -v zsh >/dev/null 2>&1; then
        zsh_directory=$(zsh -c 'printf "%s" "${ZDOTDIR:-$HOME}"')
    fi
    add_path "$zsh_directory/.zprofile"
    add_path "$zsh_directory/.zshrc"
fi
if command -v fish >/dev/null 2>&1 || [ -d "${XDG_CONFIG_HOME:-$HOME/.config}/fish" ]; then
    fish_config="${XDG_CONFIG_HOME:-$HOME/.config}/fish/conf.d"
    mkdir -p "$fish_config"
    printf '%s\n' '# Shrimply CLI PATH' 'fish_add_path --prepend --move "$HOME/.local/bin"' > "$fish_config/shrimply-cli.fish"
fi

printf 'Installed %s. Open a new terminal to use shrimply.\n' "$target"
