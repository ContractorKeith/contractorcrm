# Signing and notarization

The release workflow (`.github/workflows/release.yml`) builds and publishes a
draft release **with or without** signing certificates. Nothing here is a
prerequisite for cutting a release — add the secrets when you are ready, re-run
the workflow, and the same artifacts come back signed.

If a secret is missing, the workflow logs a `::notice::` line saying which step
was skipped and keeps going. Read the run summary to confirm what you got.

## The two-command path

1. Add the secrets you have (repository → Settings → Secrets and variables →
   Actions → New repository secret), or from a terminal:

   ```bash
   gh secret set APPLE_CERTIFICATE < apple-cert.b64
   ```

2. Re-run the release for the tag:

   ```bash
   gh workflow run release.yml -f dry_run=false
   ```

   (Or push the tag again: `git push origin :v0.1.0 && git push origin v0.1.0`.)
   The draft release is updated in place — artifacts are re-uploaded with
   `--clobber`, so signed files replace unsigned ones under the same names.

## Secrets

| Secret | Platform | Required for |
| --- | --- | --- |
| `APPLE_CERTIFICATE` | macOS | Code signing (base64 of a `.p12` export) |
| `APPLE_CERTIFICATE_PASSWORD` | macOS | Code signing (the `.p12` password) |
| `APPLE_SIGNING_IDENTITY` | macOS | Code signing (the certificate's common name) |
| `APPLE_ID` | macOS | Notarization (your Apple ID email) |
| `APPLE_PASSWORD` | macOS | Notarization (an app-specific password) |
| `APPLE_TEAM_ID` | macOS | Notarization (10-character team id) |
| `WINDOWS_CERTIFICATE` | Windows | Code signing (base64 of a `.pfx`) |
| `WINDOWS_CERTIFICATE_PASSWORD` | Windows | Code signing (the `.pfx` password) |

Signing and notarization are independent on macOS: certificate secrets alone
produce a signed but un-notarized `.dmg` (Gatekeeper still shows a warning on
first open). Add all six for a clean double-click install.

## macOS: where each value comes from

You need an Apple Developer Program membership (99 USD/year). Everything below
is done once.

**Certificate (`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
`APPLE_SIGNING_IDENTITY`)**

1. developer.apple.com → Certificates, Identifiers & Profiles → Certificates →
   `+` → **Developer ID Application**. This is the one for apps distributed
   outside the Mac App Store. Not "Mac Development", not "Mac App
   Distribution".
2. Follow the prompt to upload a Certificate Signing Request: Keychain Access →
   Certificate Assistant → Request a Certificate From a Certificate Authority →
   save to disk.
3. Download the issued `.cer` and double-click it to install into your login
   keychain.
4. In Keychain Access, find it under **My Certificates**, right-click → Export →
   `.p12`, and set a password. That password is `APPLE_CERTIFICATE_PASSWORD`.
5. Base64 the export and store it:

   ```bash
   base64 -i Certificates.p12 | pbcopy   # paste as APPLE_CERTIFICATE
   ```

6. `APPLE_SIGNING_IDENTITY` is the certificate's full name. Print it with:

   ```bash
   security find-identity -v -p codesigning
   ```

   Use the quoted string, e.g. `Developer ID Application: Keith Bloemendaal (ABCDE12345)`.

**Notarization (`APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`)**

- `APPLE_ID` — the Apple ID email on the developer account.
- `APPLE_PASSWORD` — an **app-specific password**, not your Apple ID password.
  account.apple.com → Sign-In and Security → App-Specific Passwords → generate
  one named e.g. "ContractorCRM notarization". It looks like `abcd-efgh-ijkl-mnop`.
- `APPLE_TEAM_ID` — the 10-character team id shown at
  developer.apple.com → Membership details, and in the parentheses of the
  signing identity above.

The Tauri CLI does the keychain import, signing, notarization submission, and
stapling itself once these variables are present; the workflow only forwards
them.

## macOS architecture

Releases ship **arm64 only** (`--target aarch64-apple-darwin`). Apple Silicon
has been the only Mac sold since 2023, a universal binary roughly doubles the
Rust build time and artifact size, and the x86_64 half cannot be smoke-tested on
the arm64 runner we build on — an untested slice is worse than none.

To add Intel later, in `.github/workflows/release.yml`:

- add `x86_64-apple-darwin` to the `rust_target` for the macOS matrix entry (or
  add a second matrix entry for it), and
- build with `--target universal-apple-darwin` after adding both targets to the
  `dtolnay/rust-toolchain` `targets:` list.

Rename the artifact accordingly (`macos-universal` instead of `macos-arm64`) so
existing download links stay unambiguous.

## Windows: certificate options

Windows code signing certificates are sold by certificate authorities
(DigiCert, Sectigo, SSL.com, and resellers). Two grades matter:

- **OV (organization validation)** — roughly 200–400 USD/year. Since June 2023
  the private key must live on a FIPS-140 hardware token or in a cloud HSM, so a
  plain `.pfx` file is generally no longer issued. Using one from CI means the
  CA's cloud signing service (e.g. DigiCert KeyLocker, SSL.com eSigner) and a
  different set of secrets than the two below. **An OV certificate does not
  clear SmartScreen immediately** — reputation still has to build across
  downloads.
- **EV (extended validation)** — roughly 400–700 USD/year, requires business
  verification. EV signatures get SmartScreen reputation immediately. Same
  hardware/cloud key custody rules.

The workflow's built-in path (`WINDOWS_CERTIFICATE` +
`WINDOWS_CERTIFICATE_PASSWORD`) covers the simple case: a base64-encoded `.pfx`
you hold as a file — an older OV certificate, a self-signed test certificate,
or a CA that still issues file-based keys. Produce the secret with:

```powershell
[Convert]::ToBase64String([IO.File]::ReadAllBytes("cert.pfx")) | Set-Clipboard
```

If you buy a token-based or cloud-HSM certificate, replace the "Sign the
Windows installer" step's `signtool ... /f $pfx /p ...` arguments with the CA's
documented signing command; the rest of the workflow is unchanged.

Known limitation: the workflow signs the produced NSIS installer, which is what
users download and what SmartScreen evaluates. The application `.exe` inside the
installer is not separately signed. To sign both, set
`bundle.windows.signCommand` in `src-tauri/tauri.conf.json` so the Tauri bundler
signs each binary as it packages them.

### Shipping unsigned (the default, and it is fine to start here)

An unsigned installer works. What the user sees on first run is Microsoft
Defender SmartScreen:

> **Windows protected your PC**
> Microsoft Defender SmartScreen prevented an unrecognized app from starting.
> Running this app might put your PC at risk.

They click **More info** → **Run anyway**. Put exactly that instruction, with
the SHA-256 from `SHA-256SUMS`, in the release notes and the download page so
people know the warning is expected rather than a sign something is wrong.
Verifying the hash is the real safety check either way:

```powershell
Get-FileHash .\ContractorCRM_0.1.0_windows-x64-setup.exe -Algorithm SHA256
```

macOS unsigned behaves similarly: Gatekeeper blocks the first open, and the user
right-clicks the app → Open, or clears it in System Settings → Privacy &
Security.

## Verifying a signed build

```bash
# macOS, from the mounted .dmg
codesign --verify --deep --strict --verbose=2 /Volumes/ContractorCRM/ContractorCRM.app
spctl --assess --type execute --verbose /Volumes/ContractorCRM/ContractorCRM.app
```

```powershell
# Windows
Get-AuthenticodeSignature .\ContractorCRM_0.1.0_windows-x64-setup.exe | Format-List
```

`spctl` reporting `accepted / source=Notarized Developer ID` means signing and
notarization both worked.
