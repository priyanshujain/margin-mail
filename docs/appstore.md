# Mac App Store

Margin Mail uses `studio.margin.mail`, with `studio.margin.mail.autofill` for the
email code AutoFill extension. Its starting App Store version is `0.1.0`. The App Store build uses
[tauri.appstore.conf.json](../src-tauri/tauri.appstore.conf.json). Updates open
the Mac App Store instead of downloading a replacement from GitHub.

## Local validation

Run `just build-appstore` to build an ad hoc signed sandbox bundle for this Mac.
Install it with `just _install-macos`. This build does not include AutoFill,
which needs Apple distribution profiles. It is for sandbox testing and cannot
be uploaded to Apple.

The sandbox stores mail and settings in its own container. Switching from the
direct download requires connecting accounts again; existing data is not migrated.
Check sign-in, relaunch with a connected account, sync, sending, notifications,
attachment selection and saving, exports and state import in the installed app.
The browser fixture tests cannot verify Apple sandbox permissions.

## Signed package

Run `just package-appstore` with these environment variables:

- `APPLE_TEAM_ID`: the Apple Developer team identifier.
- `MAS_APP_IDENTITY`: the Mac App Distribution application signing identity.
- `MAS_INSTALLER_IDENTITY`: the Mac Installer Distribution signing identity.
- `MAS_PROVISION_PROFILE`: the Mac App Store profile for `studio.margin.mail`.
- `MAS_AUTOFILL_PROVISION_PROFILE`: the Mac App Store profile for the extension.
- `MAS_BUILD_NUMBER`: a positive build number not previously uploaded to Apple.

Both profiles must authorize AutoFill and the shared application group
`TEAM_ID.studio.margin.mail`. The Developer ID profiles used by `just install`
are for direct distribution and cannot be used here. Packaging embeds the profiles,
signs the extension before the host, verifies both signatures and produces
`target-mas/Margin Mail.pkg`. Signing certificates must already be in the keychain.
The package command does not upload or submit anything.

Set `MAS_TARGET=universal-apple-darwin` to package both Apple Silicon and Intel;
both Rust targets must be installed. The extension is built for both architectures.
The manual [App Store workflow](../.github/workflows/appstore.yml) does this in CI
and saves the signed package as an artifact. It requires an explicit commit or tag.
Configure the signing identities and team above as repository secrets, plus
`GOOGLE_CREDENTIALS`, `MAS_APP_CERTIFICATE`, `MAS_APP_CERTIFICATE_PASSWORD`,
`MAS_INSTALLER_CERTIFICATE`, `MAS_INSTALLER_CERTIFICATE_PASSWORD`,
`MAS_PROVISION_PROFILE` and `MAS_AUTOFILL_PROVISION_PROFILE`. Certificates and
profiles use base64-encoded contents. Certificate passwords use plain secret values.
The workflow does not upload to Apple.

## Listing and review

Draft listing fields live in [appstore/metadata](../appstore/metadata).
Run `node scripts/appstore-check.mjs` for draft validation, or add `--submission`
to check the outstanding submission inputs. Capture listing images with
`node scripts/appstore-screenshots.mjs`; they render standalone HTML frames with
fictional mail, following Margin's listing layout. The app window is drawn in
HTML and CSS so its text stays sharp at the store image size.

Supply review contact details and a working reviewer mailbox before submission.
Confirm the Google OAuth client allows the reviewer to sign in and is ready for
public Gmail access. The build embeds `google-credentials.json`; example credentials
cannot be used for a submission.

Complete Apple's privacy and age rating questionnaires against the shipped app.
Mail talks directly to the configured provider; optional encrypted backup talks
to the user's Google Drive. Do not copy Margin's encryption exemption: Margin Mail
also uses XChaCha20-Poly1305 and Argon2 for stored credentials and backup. Determine
the applicable export compliance declaration before setting
`ITSAppUsesNonExemptEncryption` or submitting a build.

The [review notes](../appstore/metadata/review_notes.txt) explain the temporary
Google OAuth listener on `127.0.0.1`, the network client entitlement and Downloads
access. User-selected file access covers attachments selected or dropped by the
user. AutoFill shares recent email codes with its extension through the app group.

Register the two App IDs, enable their capabilities, create the App Store profiles
and create the macOS App Store Connect record before signing for submission.
Uploading the package, changing the Apple account and submitting for review each
require separate authorization.

Apple's requirements are described in the [Tauri App Store guide](https://v2.tauri.app/distribute/app-store/),
[App Review Guidelines](https://developer.apple.com/app-store/review/guidelines/) and
[export compliance guide](https://developer.apple.com/help/app-store-connect/manage-app-information/overview-of-export-compliance/).
