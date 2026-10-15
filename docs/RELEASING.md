# Releasing

Versions follow SemVer. **One version number** lives in `package.json`, which
`tauri.conf.json` reads, and in `src-tauri/Cargo.toml`. `release.yml` refuses to run if they
disagree with the tag.

## Cutting a release

```bash
git checkout main && git pull
./scripts/bump-version.sh 0.2.0
# edit CHANGELOG.md: move the "Unreleased" items under "## [0.2.0] - YYYY-MM-DD"
git commit -am "release: v0.2.0"
git tag v0.2.0
git push && git push origin v0.2.0
```

Then:

1. **Actions → Release** builds on macOS (universal, signed and notarized), Windows and
   Linux. It takes about 15–25 minutes; notarization is the slow part.
2. **Releases** now has a **draft** "NoExplorer v0.2.0" with these assets:
   - `NoExplorer_0.2.0_universal.dmg`
   - `NoExplorer_0.2.0_x64-setup.exe`, `NoExplorer_0.2.0_x64_en-US.msi`
   - `NoExplorer_0.2.0_amd64.AppImage`, `.deb`, `.rpm`
3. Smoke-test the `.dmg`. It should open with no Gatekeeper warning. Paste the CHANGELOG
   section into the release notes and click **Publish**.
4. The website picks up the new version immediately. It reads `releases/latest` at page load.

If a build fails, fix it on `main` and move the tag:
`git tag -f v0.2.0 && git push -f origin v0.2.0`. Delete the broken draft first.

## One-time setup: macOS signing and notarization

The workflow reads six repository secrets (**Settings → Secrets and variables → Actions**).
Without them, the macOS build still succeeds but is **unsigned**.

| Secret | What it is | How to get it |
|---|---|---|
| `APPLE_CERTIFICATE` | Base64 of your **Developer ID Application** certificate + key (`.p12`) | Keychain Access → My Certificates → right-click "Developer ID Application: …" → Export → `cert.p12` with a password, then `base64 -i cert.p12 \| pbcopy` |
| `APPLE_CERTIFICATE_PASSWORD` | The password you set on export | |
| `APPLE_SIGNING_IDENTITY` | The full certificate name | `security find-identity -v -p codesigning` → e.g. `Developer ID Application: VNAPPMOB CO., LTD (ABCDE12345)` |
| `APPLE_ID` | Apple ID email of the developer account | |
| `APPLE_PASSWORD` | An **app-specific password**, not your Apple ID password | <https://account.apple.com> → Sign-In and Security → App-Specific Passwords |
| `APPLE_TEAM_ID` | 10-character team id | <https://developer.apple.com/account> → Membership details |

No Developer ID certificate yet? Go to developer.apple.com → Certificates → **+** →
"Developer ID Application" → upload a CSR made in Keychain Access → download the
certificate and double-click to install it. Then export it as above.

To check a downloaded build:

```bash
spctl -a -vvv -t install /Applications/NoExplorer.app   # → "accepted, source=Notarized Developer ID"
```

## Windows signing (not set up yet)

Unsigned installers trigger a SmartScreen warning ("More info → Run anyway"). Options are
[SignPath Foundation](https://signpath.org) (free for OSS) or an OV/EV certificate. Tauri
docs: <https://v2.tauri.app/distribute/sign/windows/>. Tracked in the [roadmap](ROADMAP.md).

## Website

- The source is `/site`, a static page with no build step. `pages.yml` deploys it on every
  push to `main` that touches `site/**`.
- One-time setup:
  1. Repo **Settings → Pages → Source: GitHub Actions**.
  2. In DNS for `vnappmob.com`, add `CNAME noexplorer → vnappmob.github.io`.
  3. Back in Settings → Pages, set the custom domain to `noexplorer.vnappmob.com`
     (`site/CNAME` already contains it) and tick **Enforce HTTPS** once the certificate is issued.
- The download buttons call `https://api.github.com/repos/vnappmob/noexplorer/releases/latest`
  and pick the asset by file extension. If the repo is renamed, update `REPO` in `site/app.js`.
