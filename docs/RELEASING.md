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
# The org blocks the workflow's GITHUB_TOKEN from *creating* releases (403 "Resource not
# accessible by integration"), so create the draft yourself; tauri-action then uploads into it:
gh release create v0.2.0 --draft --verify-tag --title "NoExplorer v0.2.0" --notes "See CHANGELOG.md"
```

Then:

1. **Actions → Release** builds on macOS (universal; the `.app` and the `.dmg` are both signed,
   notarized and stapled), Windows and Linux. It takes about 15–25 minutes; notarization is the slow part.
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
spctl -a -vvv -t exec /Applications/NoExplorer.app                                   # → accepted, source=Notarized Developer ID
spctl -a -vvv -t open --context context:primary-signature NoExplorer_X.Y.Z_universal.dmg   # → accepted (from v0.2.0)
xcrun stapler validate NoExplorer_X.Y.Z_universal.dmg
```

## Auto-update signing key

Installed apps verify each update with the public key in `tauri.conf.json`
(`plugins.updater.pubkey`). The workflow signs with:

| Secret | Value |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | Contents of `~/.tauri/noexplorer-updater.key` (generated 2026-10-15) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Contents of `~/.tauri/noexplorer-updater.password` |

**Back up both files** (for example in a password manager). Without the private key no new
version can be delivered to existing installs, and every user would have to reinstall by hand.
Never commit them.

An update reaches users when the release is **published**: apps poll
`releases/latest/download/latest.json`, which ignores drafts. Check that `latest.json` is among
the assets before publishing.

## Windows signing (not set up)

Windows installers are **unsigned**, so SmartScreen warns on first install ("More info → Run
anyway"). Auto-updates are still verified with the updater key. SignPath Foundation's free
certificate was declined on 2026-10-17 (ADR-030), and the maintainer accepts unsigned Windows
builds until there's budget for a certificate.

When that day comes:
- Options: an OV/EV certificate (hardware token or cloud HSM, e.g. Certum, Sectigo, SSL.com
  eSigner) or Azure Artifact Signing if the account becomes eligible (US/Canada individuals, or
  organizations in the US, Canada, EU or UK).
- Tauri signs during `tauri build` through `bundle.windows` (`certificateThumbprint` for a
  certificate in the runner's store, or `signCommand` for a cloud signer). The updater
  signatures are then made from the signed files automatically. Guide:
  <https://v2.tauri.app/distribute/sign/windows/>.
- Update the website's [code signing page](../site/src/content/docs/docs/code-signing.md), the FAQ and the
  README note.

## Website

- The source is `/site`, an Astro project (see below). The build output `site/dist` is served by
  **Cloudflare Workers static assets** (no Worker code), with `site/public/_headers` (security and
  cache headers) and `404.html`. The config is `wrangler.jsonc`.
- **Cloudflare Workers Builds** (Cloudflare's GitHub integration) deploys on every push to
  `main`. It runs `npx wrangler deploy`, which reads `wrangler.jsonc`. Pull requests get a
  preview URL. Nothing lives in GitHub Actions and no secrets are needed. Build logs are in
  the Cloudflare dashboard → Workers & Pages → `noexplorer-site` → Deployments.
- The custom domain `noexplorer.vnappmob.com` is declared in `wrangler.jsonc`
  (`custom_domain: true`). Because `vnappmob.com` is a Cloudflare zone, the first deploy
  creates the DNS record and the TLS certificate automatically.
- One-time setup in the Cloudflare dashboard (the account that owns `vnappmob.com`):
  1. **Workers & Pages → Create → Import a repository** → connect GitHub and grant the
     Cloudflare app access to `vnappmob/noexplorer`.
  2. Worker name: **`noexplorer-site`** (it must match `name` in `wrangler.jsonc`).
  3. Build command: *(leave empty: `wrangler deploy` builds the site itself)*. Deploy command:
     `npx wrangler deploy` (the default).
     Root directory: `/`.
  4. Optional, under Settings → Build → Build watch paths: include `site/*` and
     `wrangler.jsonc`, so app-only commits don't trigger a deploy.
  5. Save and deploy, then open <https://noexplorer.vnappmob.com>.
- **The site is an Astro project in `site/`** (its own `package.json`, nothing shared with the app):
  the home page is `site/src/pages/index.astro` (static assets in `site/public/`), the docs are
  Markdown in `site/src/content/docs/docs/` rendered by Starlight (search, sidebar from each page's
  `sidebar.order`, sitemap). `npm run site:build` builds `site/dist`; CI builds it too.
- `wrangler deploy` runs the build itself (`build.command` in `wrangler.jsonc`), so Cloudflare's
  Workers Builds needs no build settings: its default deploy command does everything.
- Manual deploy from a laptop, if ever needed: `npx wrangler login && npm run site:deploy`.
- The download buttons call `https://api.github.com/repos/vnappmob/noexplorer/releases/latest`
  and pick the asset by file extension. If the repo is renamed, update `REPO` in `site/public/app.js`.
