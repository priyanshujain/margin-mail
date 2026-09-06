#!/usr/bin/env bash
# Build the flatpak from the deb sitting beside this script as margin-mail.deb, and write a
# single-file bundle next to it. Used by both CI jobs and by hand; the only difference between the
# two is where the deb came from.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
cd "$here"

id=studio.margin.mail
deb=margin-mail.deb
bundle=${1:-$here/margin-mail.flatpak}

[ -f "$deb" ] || { echo "flatpak/build.sh: no $deb beside this script." >&2; exit 1; }

runtime_version=$(sed -n "s/^runtime-version: *'\(.*\)'/\1/p" "$id.yml")

# --user so nothing here needs root, and --if-not-exists so a second run is free.
flatpak remote-add --user --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
flatpak install --user --noninteractive flathub \
  org.flatpak.Builder \
  "org.gnome.Platform//$runtime_version" \
  "org.gnome.Sdk//$runtime_version"

rm -rf build repo
# Ubuntu 22.04's builder calls appstream-compose, which GNOME 48 no longer ships.
# Use the Flathub builder for its current AppStream support on CI and local builds alike.
flatpak run org.flatpak.Builder --user --disable-rofiles-fuse --force-clean --repo=repo build "$id.yml"
flatpak build-bundle repo "$bundle" "$id"

echo "Wrote $bundle"
