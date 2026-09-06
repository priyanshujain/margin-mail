# Installing Margin Mail

Every build comes from the same release. Pick the file for your machine from
[the latest release](https://github.com/priyanshujain/margin-mail/releases/latest) and follow the
section below for it. The version number in the file names changes with every release; `0.1.0`
stands in for it throughout.

Whatever you install, the app keeps its mirror of your mail, your accounts and every decision
you have ever made about a sender in one directory, and uninstalling never touches it. Where that
directory is, and how to move it, is at the end.

## macOS

Apple Silicon and Intel are the same file: the dmg carries a universal binary.

1. Download `Margin.Mail_0.1.0_universal.dmg`.
2. Open it and drag **Margin Mail** onto the Applications folder in the same window.
3. Eject the disk image and open the app from Applications or Spotlight.

The bundle is signed with a Developer ID and notarized, so it opens on the first double click with
no right-click-and-Open dance and no trip to System Settings.

macOS asks about notifications the first time the app has something to tell you rather than at
launch. If you say no and change your mind, it is under Notifications in System Settings, and the
app's Settings has a button that takes you straight there.

The minimum is macOS 10.15.

## Linux

Four packages, and they are not equal. If you are on Wayland, or on NixOS, use Nix. Otherwise the
deb on Debian and Ubuntu, the flatpak on anything else, and the AppImage when you want no install
at all.

### deb, on Debian and Ubuntu

```
sudo apt install ./Margin.Mail_0.1.0_amd64.deb
```

`apt` rather than `dpkg -i`, because it pulls `libwebkit2gtk-4.1-0` and `libgtk-3-0` in for you.
The app then appears in your launcher. Upgrading is the same command with the newer file, and
`sudo apt remove margin-mail` reverses it.

Built on Ubuntu 22.04, so 22.04 is the oldest release it runs on. Anything older has a glibc the
binary was not linked against and will refuse to start.

### flatpak, on everything else

```
flatpak install ./Margin.Mail_0.1.0_amd64.flatpak
flatpak run studio.margin.mail
```

A single self-contained file: it brings its own GTK and WebKit, so it does not care what your
distribution ships. It is not on Flathub and will not be, because a Flathub build has to come from
source and this app's Google client is embedded at compile time from a file that is deliberately
not in the repository.

The sandbox is narrow on purpose. The app gets the network, the notification service and your
downloads directory, and nothing else. That means saved attachments and exports land in
`~/Downloads` and can go nowhere else, and the app's data lives under
`~/.var/app/studio.margin.mail/` rather than in the usual place, so a flatpak install and a deb
install do not see each other's mail.

`flatpak uninstall studio.margin.mail` removes it, and add `--delete-data` to take the mail with
it.

### Nix

The package relinks the published deb against nixpkgs' own GTK and WebKit, which is the only build
here that runs as a native Wayland client rather than falling back to Xwayland.

```
NIXPKGS_ALLOW_UNFREE=1 nix run --impure github:priyanshujain/margin-mail#margin-mail
```

`NIXPKGS_ALLOW_UNFREE` because the licence is FSL rather than MIT, so nix asks first. To keep it,
add the flake as an input and the overlay to your configuration:

```nix
{
  inputs.margin-mail.url = "github:priyanshujain/margin-mail";

  # In your nixpkgs configuration:
  nixpkgs.overlays = [ inputs.margin-mail.overlays.default ];
  nixpkgs.config.allowUnfreePredicate = pkg: builtins.elem (lib.getName pkg) [ "margin-mail" ];
  environment.systemPackages = [ pkgs.margin-mail ];
}
```

A Nix install does not update itself. The store is read only, so the app reports the new version
and tells you to update the flake instead of trying to replace its own binary.

### AppImage, when you want no install

```
chmod +x Margin.Mail_0.1.0_amd64.AppImage
./Margin.Mail_0.1.0_amd64.AppImage
```

Nothing is written outside your home directory and there is nothing to uninstall. It carries
Ubuntu's GTK stack, which cannot talk to a modern Wayland compositor, so on Wayland it runs through
Xwayland and looks slightly soft on a HiDPI screen. That is the trade for a single portable file.

To get it into your launcher rather than running it from a terminal, `just install` from a clone
does the whole thing, or by hand:

```
install -Dm755 Margin.Mail_0.1.0_amd64.AppImage ~/.local/bin/margin-mail
```

and write a `.desktop` file pointing `Exec` at it.

## Windows

Two installers, and either is fine. The `.exe` is the friendlier one; the `.msi` is what you want
if you are deploying by policy.

1. Download `Margin.Mail_0.1.0_x64-setup.exe`.
2. Run it. Windows SmartScreen will say it does not recognise the publisher, because the installer
   is not code-signed. Click **More info**, then **Run anyway**.
3. The app appears in the Start menu.

Uninstalling is through Apps in Settings, the same as anything else.

Windows 10 1803 or newer, and WebView2, which every supported Windows already has.

## Updating

Every install except Nix and the flatpak updates itself. The app checks the release feed on launch,
tells you when something newer is out, and installs it when you say so. **Check for Updates** in
the File menu, or the application menu on macOS, asks immediately.

Nix updates with the flake. The flatpak updates with `flatpak update studio.margin.mail` once you
have installed the newer bundle file, since a single-file bundle carries no remote to check.

## Where your mail lives

One directory per platform, and it survives uninstalling:

- macOS: `~/Library/Application Support/studio.margin.mail`
- Linux: `~/.local/share/studio.margin.mail`, or `~/.var/app/studio.margin.mail/data/studio.margin.mail` under flatpak
- Windows: `%APPDATA%\studio.margin.mail`

It holds the mirror of your mail, the state database of every decision you have made, and your
sealed refresh tokens. Copying it to another machine moves everything except the tokens, which are
sealed against the machine that stored them, so the accounts ask to be connected again and nothing
else changes.

To remove the app and its mail together, uninstall and then delete that directory.

## Building it yourself

`just install` from a clone builds for the machine you are sitting at and puts it where that
machine expects to find applications. [release.md](release.md) has what a build needs and how a
release is cut.
