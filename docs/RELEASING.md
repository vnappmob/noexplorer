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

## Windows signing (SignPath, waiting for approval)

Free code signing from [SignPath Foundation](https://signpath.org) for open-source projects
(ADR-030). The certificate belongs to SignPath Foundation, so Windows shows it as the publisher.
`release.yml` already contains the flow; it stays off (unsigned installers, exactly as before)
until the repo variable `SIGNPATH_ORGANIZATION_ID` is set.

**One-time setup, after SignPath approves the project:**
1. In SignPath: add **GitHub.com as a Trusted Build System** and link it to the project. Create two
   artifact configurations with the slugs `app-exe` and `installers`, pasting
   [`.signpath/app-exe.xml`](../.signpath/app-exe.xml) and
   [`.signpath/installers.xml`](../.signpath/installers.xml). The signing policy (default slug
   `release-signing`) must require manual approval.
2. Create a SignPath API token for a user with submitter rights.
3. In GitHub → Settings → Secrets and variables → Actions:
   - secret `SIGNPATH_API_TOKEN`;
   - variable `SIGNPATH_ORGANIZATION_ID`; optional `SIGNPATH_PROJECT_SLUG` (default `noexplorer`)
     and `SIGNPATH_POLICY_SLUG` (default `release-signing`).
4. Turn on MFA for everyone on the team, on GitHub and SignPath (a SignPath Foundation rule).

**Each release** (once enabled), the `windows` job pauses twice for approval in SignPath. You
have an hour for each:
1. `noexplorer.exe` (so the installed app is signed too), then the installers are built around it;
2. the `-setup.exe` and `.msi` installers.

Signing changes the files, so the job makes the updater signatures (`*.sig`) again from the
signed installers, and the `windows-updater-json` job adds the Windows entries to `latest.json`
(`scripts/updater-json-windows.mjs`). Check after publishing: on a Windows machine, the
installer's Properties → Digital Signatures shows SignPath Foundation.

The [code signing policy](https://noexplorer.vnappmob.com/docs/code-signing) on the website
lists the team roles. Keep it current when people join or leave.

## Website

- The source is `/site`: static HTML/CSS/JS (home page `index.html` + `app.js`; docs pages in `site/docs/`), `_headers` (security and cache headers) and
  `404.html`. It is served by **Cloudflare Workers static assets** (no Worker code). The
  config is `wrangler.jsonc`.
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
  3. Build command: *(leave empty)*. Deploy command: `npx wrangler deploy` (the default).
     Root directory: `/`.
  4. Optional, under Settings → Build → Build watch paths: include `site/*` and
     `wrangler.jsonc`, so app-only commits don't trigger a deploy.
  5. Save and deploy, then open <https://noexplorer.vnappmob.com>.
- **Docs pages are generated.** Edit `site-src/docs/<page>.html` (and `_layout.html` for the
  frame), then run `npm run site:docs` and commit both. `npm run check` fails when `site/docs/`
  is stale. New pages go in `PAGES` in `scripts/build-docs.mjs` (sidebar order).
- Manual deploy from a laptop, if ever needed: `npx wrangler login && npm run site:deploy`.
- The download buttons call `https://api.github.com/repos/vnappmob/noexplorer/releases/latest`
  and pick the asset by file extension. If the repo is renamed, update `REPO` in `site/app.js`.
