# The Linux package: the deb the release workflow published, relinked against nixpkgs' own GTK and
# WebKit so it runs as a native Wayland client. It is a binary package by necessity: the Google
# OAuth client is embedded at compile time from a file that is deliberately not in the repo, so
# anything built from source here would run and then say Google is not set up.
#
# Ported from Margin Calendar's nix/package.nix, which is the sibling to read when this needs work.
{
  lib,
  stdenv,
  fetchurl,
  dpkg,
  autoPatchelfHook,
  wrapGAppsHook3,
  cairo,
  dbus,
  gdk-pixbuf,
  glib,
  glib-networking,
  gsettings-desktop-schemas,
  gtk3,
  libsoup_3,
  mesa,
  runtimeShell,
  webkitgtk_4_1,
  xdg-utils,
}:

let
  release = lib.importJSON ./release.json;
in
stdenv.mkDerivation {
  pname = "margin-mail";
  inherit (release) version;

  src = fetchurl {
    url = "https://github.com/priyanshujain/margin-mail/releases/download/v${release.version}/Margin.Mail_${release.version}_amd64.deb";
    inherit (release) hash;
  };

  unpackPhase = ''
    runHook preUnpack
    dpkg-deb -x $src .
    runHook postUnpack
  '';

  nativeBuildInputs = [
    dpkg
    autoPatchelfHook
    wrapGAppsHook3
  ];

  buildInputs = [
    cairo
    dbus
    gdk-pixbuf
    glib
    glib-networking
    gsettings-desktop-schemas
    gtk3
    libsoup_3
    webkitgtk_4_1
  ];

  # The binary sits under lib/ so wrapGAppsHook wraps the launcher in bin/ and nothing else.
  #
  # Outside NixOS there is no /run/opengl-driver, so the libglvnd this build links finds no EGL
  # driver and WebKit aborts its web process on the spot. The launcher points it at nixpkgs' Mesa
  # instead, unless something like nixGL already did. The xdg-open shim strips that again for the
  # browser the app opens for the Google consent page, which has a Mesa of its own.
  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin $out/lib/margin-mail/bin
    cp usr/bin/margin-mail $out/lib/margin-mail/
    cp -r usr/share $out/
    mv "$out/share/applications/Margin Mail.desktop" $out/share/applications/margin-mail.desktop

    cat > $out/bin/margin-mail <<LAUNCHER
    #!${runtimeShell}
    if [ ! -d /run/opengl-driver ]; then
      : "\''${__EGL_VENDOR_LIBRARY_FILENAMES:=$(echo ${mesa}/share/glvnd/egl_vendor.d/*.json)}"
      : "\''${LIBGL_DRIVERS_PATH:=${mesa}/lib/dri}"
      : "\''${GBM_BACKENDS_PATH:=${mesa}/lib/gbm}"
      export __EGL_VENDOR_LIBRARY_FILENAMES LIBGL_DRIVERS_PATH GBM_BACKENDS_PATH
    fi
    exec $out/lib/margin-mail/margin-mail "\$@"
    LAUNCHER

    cat > $out/lib/margin-mail/bin/xdg-open <<SHIM
    #!${runtimeShell}
    unset __EGL_VENDOR_LIBRARY_FILENAMES LIBGL_DRIVERS_PATH GBM_BACKENDS_PATH
    exec ${xdg-utils}/bin/xdg-open "\$@"
    SHIM

    chmod +x $out/bin/margin-mail $out/lib/margin-mail/bin/xdg-open
    runHook postInstall
  '';

  # The variable is how the app knows the store owns the binary, so "Check for updates" reports the
  # new version and the upgrade command instead of trying to replace a file it cannot write.
  preFixup = ''
    gappsWrapperArgs+=(
      --prefix PATH : $out/lib/margin-mail/bin
      --set MARGIN_MAIL_PACKAGED_BY nix
    )
  '';

  meta = {
    description = "A calm, keyboard-first mail client for Gmail";
    longDescription = "New senders wait at the door until you let them in, people and newsletters and receipts live in three separate boxes, and every decision you make is kept beside the mail rather than inside it.";
    homepage = "https://github.com/priyanshujain/margin-mail";
    # Not one of lib.licenses: FSL is source available rather than free, and it turns MIT two years
    # after each release. `free = false` is what makes nix ask before building it, which is correct
    # and is why the workflows pass NIXPKGS_ALLOW_UNFREE.
    license = {
      shortName = "FSL-1.1-MIT";
      fullName = "Functional Source License, Version 1.1, MIT Future License";
      url = "https://github.com/priyanshujain/margin-mail/blob/main/LICENSE";
      free = false;
      redistributable = true;
    };
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
    platforms = [ "x86_64-linux" ];
    mainProgram = "margin-mail";
  };
}
