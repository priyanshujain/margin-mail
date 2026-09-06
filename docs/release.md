# Releasing

## Installing locally

`just install` builds the app for whatever machine you are sitting at and puts it where that
machine expects to find applications: `/Applications` on macOS, or the package manager on Linux.
It is the same command whether or not the app is already installed, so it doubles as the update.
On macOS it asks a running copy to quit first, because replacing a bundle under a live process
leaves it half old and half new, and starts the new one once it is in place, so the copy on screen
is never older than the copy installed. `just uninstall` reverses it and leaves the data directory
alone, which matters more here than it does in the siblings: that directory holds the state
database, and the state database is every decision you have ever made about a sender.

The local build skips the dmg and builds only the `.app`, since nothing about copying a bundle into
place needs a disk image and building one is the slowest part of a mac bundle. A locally installed
app has no updater artifacts, so it will not update itself. Rerun `just install`.

## Signing

macOS shows no notifications from an app whose bundle is not signed, and `tauri build` on its own
leaves only the linker's signature on the binary, which does not count: the app never appears under
Notifications in System Settings and is never asked. So `tauri.conf.json` names `-` as the signing
identity and every macOS build is at least ad-hoc signed as a bundle, which is enough for
notifications. A real identity replaces that wherever one is available. `just build` sources
`~/.margin-signing/studio.margin.app.env` when it exists (another directory with
`MARGIN_SIGNING_DIR`), which exports `APPLE_SIGNING_IDENTITY`, and Tauri takes that over the config.
The release workflow does the same from repository secrets and notarizes when the App Store Connect
key is there too:

- `APPLE_CERTIFICATE` and `APPLE_CERTIFICATE_PASSWORD`, the Developer ID Application certificate as
  a base64 `.p12` and its password. Tauri imports it into a temporary keychain for the build.
- `APPLE_SIGNING_IDENTITY` and `APPLE_TEAM_ID`, the identity's name and the team behind it.
- `APPLE_API_KEY`, `APPLE_API_ISSUER` and `APPLE_API_KEY_P8`, the App Store Connect API key id, its
  issuer and the contents of the `.p8`. Without these three the bundle is signed and not notarized,
  which Gatekeeper minds on a download and notifications do not.

From the signing directory that is:

```
cd ~/.margin-signing
gh secret set APPLE_CERTIFICATE < <(base64 -i developer-id.p12)
gh secret set APPLE_CERTIFICATE_PASSWORD < developer-id.p12.pass
gh secret set APPLE_SIGNING_IDENTITY --body "Developer ID Application: <name> (<team>)"
gh secret set APPLE_TEAM_ID --body "<team>"
gh secret set APPLE_API_KEY --body "<key id>"
gh secret set APPLE_API_ISSUER --body "<issuer>"
gh secret set APPLE_API_KEY_P8 < AuthKey.p8
```

## Before the first release

Two things have to exist that do not yet.

**The updater key.** Done. The pair was generated with
`pnpm tauri signer generate -w ~/.tauri/margin_mail_updater.key`, the public half is in
`src-tauri/tauri.release.conf.json` and the private half and its password are the repository's
`TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secrets. The private half and
its password sit beside the siblings' in `~/.tauri`, and only there. The public half is baked into
every build, so the private half can never be rotated without stranding everyone who has not
updated yet: back it up somewhere that is not the machine that made it.

**The verification.** `gmail.modify` and `gmail.settings.basic` are restricted scopes, so the shared
Cloud project has to pass restricted scope verification before this app is anything other than a
hundred lifetime users behind an unverified consent screen. That is a form, a privacy policy that is
true, a demo video and a written justification per scope, and it is per project rather than per
client, so it covers margin and Margin Calendar too and a lapse blocks all three. The hedge is in
[architecture.md](architecture.md): Gmail over IMAP and SMTP with an app password needs no project,
no verification and has no cap.

## Cutting a release

Releases are manual: run the **Release** workflow from the Actions tab. Leave the version empty to
bump the patch number, or give one to set it. The workflow bumps `tauri.conf.json`, `package.json`
and `Cargo.toml` together, commits that to main, tags it, and builds the tag rather than whatever
main happens to be by then.

Three runners build in parallel: a universal macOS bundle, an x86_64 Linux one and an x86_64
Windows one. Nothing is published until all three have landed. The publish job downloads
`latest.json` and refuses to take the release out of draft unless `darwin-aarch64`,
`darwin-x86_64`, `linux-x86_64` and `windows-x86_64` are all in it. A half-populated manifest is
worse than no release: the updater would offer an update to the platforms that made it and error on
the ones that did not.

Linux builds on Ubuntu 22.04 on purpose. The bundle will not run on anything older than the glibc it
was linked against, so it is built on the oldest release that is supported.

Before bundling on Linux, `scripts/prepare-bundle.mjs` prepares a pinned GTK packaging plugin in
the project's tools cache. It leaves Wayland libraries to the host, alongside the host's graphics
driver: bundling Ubuntu's older Wayland makes recent Mesa fail to load and leaves the AppImage
window blank. This happens before Tauri generates updater signatures.

The Windows installers are not code-signed, so SmartScreen warns on the first download until the
app has built up reputation. A certificate would go in as `WINDOWS_CERTIFICATE` and
`WINDOWS_CERTIFICATE_PASSWORD` and needs nothing else changed.

Phones do not come from this pipeline at all. The store is their update channel.

## The two Linux packages Tauri does not build

Tauri produces the deb and the AppImage. The flatpak and the Nix package are built from the deb
afterwards, by two more jobs, and [install.md](install.md) says which of the four a reader should
actually pick.

The **flatpak** job runs between the build and the publish, so a release never goes out with the
Linux artifacts half there. It downloads the deb from the draft release, runs
`flatpak/build.sh` over the manifest in the same directory, and uploads a single-file bundle
beside it. The manifest is hand-written because Tauri has no flatpak bundler, and it pins
`org.gnome.Platform` 48, which is the newest runtime that still carries the GTK3 WebKit wry links
against. The sandbox gets the network, the notification service and the downloads directory, and
nothing else. CI builds the same manifest on every push to main, against a deb built there, which
is the only way a break in it gets found before a release.

On Linux, `just flatpak` builds the deb and repackages it locally. Install `flatpak` and
`flatpak-builder` 1.4.4 or newer first; the script installs the GNOME runtime for your user.
CI gets these tools from the Flatpak team's stable PPA because Ubuntu 22.04's original builder
calls `appstream-compose`, which the GNOME 48 SDK no longer includes.

The **nix** job runs after the publish, so the flake can only ever point at a release that survived
the manifest check. It hashes the published deb into `nix/release.json`, builds the package to
prove the pin works, and commits the pin to main. `NIXPKGS_ALLOW_UNFREE` and `--impure` are in that
command because FSL is not a free licence and `nix/package.nix` says so honestly rather than
claiming MIT to dodge the prompt.

Adding a Homebrew cask is the one distribution route the siblings have and this does not. It is a
job at the end of the release workflow, a `Casks/margin-mail.rb` in `priyanshujain/homebrew-margin`
and a `HOMEBREW_TAP_DEPLOY_KEY` secret; margin's `release.yml` has the job to copy.

## What the build needs

Both jobs check out this repository and the public margin repository side by side, because
`package.json` depends on `margin-shared` through a relative path. A token edited there is meant to
show up in every Margin app at once, and a copy vendored here would defeat that.

Ten repository secrets, all of them set. The three below, and the seven Apple ones under Signing
above:

- `GOOGLE_CREDENTIALS`, the contents of the real `google-credentials.json`. The build writes it to
  the repository root and `build.rs` embeds it. Without it the build falls back to the example file
  and warns, which produces an app that runs and then says it is not set up yet. It is the same
  value in all three Margin repositories, because it is the same OAuth client.
- `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, which sign the updater
  artifacts.

The OAuth client secret ends up inside the shipped binary. That is how installed apps work and
Google does not treat it as confidential: an installed client cannot keep a secret, which is why the
flow uses PKCE and why the token exchange is safe without one.

## Updates

Installed copies check
`https://github.com/priyanshujain/margin-mail/releases/latest/download/latest.json` and update
themselves from it. `--latest` on the publish step is what moves that pointer, so a release that
fails the manifest check stays a draft and nobody is offered a broken update.

A packaged install does not update itself. The Nix wrapper in the siblings sets a
`PACKAGED_BY` variable and the app checks for it before it would touch its own binary, which in a
read-only store it could not replace anyway; this app reads `MARGIN_MAIL_PACKAGED_BY` for the same
reason, and reports the newer version and the upgrade command instead.
