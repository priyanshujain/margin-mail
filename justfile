# Building and installing Margin Mail on the machine you are sitting at. The release pipeline in
# .github/workflows/release.yml is what builds for everyone else; this is the local equivalent, and
# `just install` is deliberately the same command whether or not the app is already installed.

set shell := ["bash", "-euo", "pipefail", "-c"]

app := "Margin Mail"
bundle := "src-tauri/target/release/bundle"

# List the recipes.
default:
    @just --list

# Run the app against the Vite dev server.
dev:
    pnpm tauri dev

# The gate: the frontend suites and the Rust suites, both of which must be green.
test:
    pnpm test
    cd src-tauri && cargo test

# The browser suite that drives the real UI, including the screenshots of every screen.
test-ui:
    pnpm test:ui

# Recapture the pictures the guide ships with, from the fixture rather than from anybody's mailbox.
# They are committed, so this is not part of `test-ui`: an ordinary run must not rewrite the tree.
guide-shots:
    GUIDE_SHOTS=1 pnpm exec playwright test tests/guide-shots.spec.ts

# The prose gate: no em dashes, no broken relative links, no directory trees.
docs:
    node scripts/docs-check.mjs

# Build the release bundle for this machine.
build:
    #!/usr/bin/env bash
    set -euo pipefail
    # The .app on macOS, the .deb and AppImage on Linux. No dmg: nothing here needs a disk image to
    # copy a bundle into place, and building one is the slowest part of a mac bundle.
    pnpm install
    # macOS shows no notifications from a bundle that is not signed, so tauri.conf.json ad-hoc signs
    # at minimum and a real identity replaces that when this machine has one: the signing directory
    # holds an env file naming it, and Tauri reads APPLE_SIGNING_IDENTITY over the config.
    if [ "$(uname -s)" = Darwin ]; then
      signing="${MARGIN_SIGNING_DIR:-$HOME/.margin-signing}/studio.margin.app.env"
      if [ -f "$signing" ]; then
        set -a; . "$signing"; set +a
        echo "Signing as $APPLE_SIGNING_IDENTITY"
      else
        echo "No $signing; the bundle will be ad-hoc signed."
      fi
    fi
    case "$(uname -s)" in
      Darwin) pnpm tauri build --bundles app ;;
      Linux)  pnpm tauri build --bundles deb,appimage ;;
      *)      echo "just: no local build for $(uname -s); macOS and Linux are the desktop targets." >&2; exit 1 ;;
    esac

# Build, install over whatever version is already installed, and start the new one.
install: build
    #!/usr/bin/env bash
    set -euo pipefail
    case "$(uname -s)" in
      Darwin) just _install-macos ;;
      Linux)  just _install-linux ;;
      *)      echo "just: no installer for $(uname -s); macOS and Linux are the desktop targets." >&2; exit 1 ;;
    esac

_install-macos:
    #!/usr/bin/env bash
    set -euo pipefail
    src="{{bundle}}/macos/{{app}}.app"
    [ -d "$src" ] || { echo "just: nothing to install, $src does not exist." >&2; exit 1; }

    # /Applications is writable by admin users, so the common case needs no sudo. When it is not,
    # ~/Applications is a real Launchpad location and beats prompting for a password mid-build.
    if [ -w /Applications ]; then dir=/Applications; else dir="$HOME/Applications"; mkdir -p "$dir"; fi
    dest="$dir/{{app}}.app"

    running() { pgrep -x "{{app}}" > /dev/null || pgrep -x margin-mail > /dev/null; }

    # Replacing the bundle under a running app leaves it half old and half new, and the running copy
    # holds the deleted files open. Ask it to quit and wait, rather than killing it.
    if running; then
      echo "Quitting the running {{app}}…"
      osascript -e 'quit app "{{app}}"' 2> /dev/null || true
      for _ in $(seq 40); do running || break; sleep 0.25; done
      if running; then echo "just: {{app}} is still running; quit it and try again." >&2; exit 1; fi
    fi

    rm -rf "$dest"
    cp -R "$src" "$dest"
    version=$(defaults read "$dest/Contents/Info.plist" CFBundleShortVersionString 2> /dev/null || echo "?")
    echo "Installed $version to $dest"
    open "$dest"

_install-linux:
    #!/usr/bin/env bash
    set -euo pipefail
    deb=$(ls -t {{bundle}}/deb/*.deb 2> /dev/null | head -1 || true)
    appimage=$(ls -t {{bundle}}/appimage/*.AppImage 2> /dev/null | head -1 || true)

    if [ -n "$deb" ] && command -v apt-get > /dev/null; then
      sudo apt-get install -y --reinstall "$PWD/$deb"
      echo "Installed $deb"
    elif [ -n "$deb" ] && command -v dpkg > /dev/null; then
      sudo dpkg -i "$deb"
      echo "Installed $deb"
    elif [ -n "$appimage" ]; then
      install -Dm755 "$appimage" "$HOME/.local/bin/margin-mail"
      install -Dm644 src-tauri/icons/128x128.png "$HOME/.local/share/icons/margin-mail.png"
      mkdir -p "$HOME/.local/share/applications"
      printf '%s\n' \
        '[Desktop Entry]' \
        'Type=Application' \
        'Name=Margin Mail' \
        'Comment=A calm, keyboard-first mail client for Gmail' \
        'Exec=margin-mail' \
        'Icon=margin-mail' \
        'Categories=Office;Email;' \
        > "$HOME/.local/share/applications/margin-mail.desktop"
      echo "Installed $appimage to ~/.local/bin/margin-mail"
      echo "Make sure ~/.local/bin is on your PATH."
    else
      echo "just: nothing to install, no .deb or AppImage under {{bundle}}." >&2
      exit 1
    fi

# Remove the installed app, leaving its data directory alone.
uninstall:
    #!/usr/bin/env bash
    set -euo pipefail
    case "$(uname -s)" in
      Darwin)
        for dest in "/Applications/{{app}}.app" "$HOME/Applications/{{app}}.app"; do
          if [ -d "$dest" ]; then rm -rf "$dest"; echo "Removed $dest"; fi
        done
        ;;
      Linux)
        if dpkg -s margin-mail > /dev/null 2>&1; then sudo apt-get remove -y margin-mail; fi
        rm -f "$HOME/.local/bin/margin-mail" \
              "$HOME/.local/share/icons/margin-mail.png" \
              "$HOME/.local/share/applications/margin-mail.desktop"
        echo "Removed the AppImage install, if there was one."
        ;;
      *) echo "just: nothing to uninstall on $(uname -s)." >&2; exit 1 ;;
    esac
