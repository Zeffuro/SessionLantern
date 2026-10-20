# Session Lantern

Desktop companion for exporting a TikTok session to your bot dashboard.

1. Open TikTok sign-in and log in manually. Keep that window open.
2. The notice turns green when all four cookies are available, including on TikTok’s error page.
3. Generate a password, save the encrypted `.sessionpack`, then copy the password.
4. In the dashboard’s administrator settings, import the file and verify the bot account.

The password stays in this window until you clear it or close the app. Existing exports are never replaced; use a new file name. Cookies are checked locally after page loads; nothing is uploaded automatically. Google/Facebook may reject embedded sign-in; use TikTok’s QR login instead.

## Development

Requires Node.js 22+, Rust 1.90+ and [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```powershell
npm ci
npm run dev
npm run check
cargo test --manifest-path src-tauri/Cargo.toml
npm test
npm run bundle:windows
```

## Releases

Push a version tag matching every manifest, such as `v0.2.1`, or run **Desktop builds and releases** with that tag. CI builds Windows x64 (portable/installer), Linux x64 (AppImage/deb), and macOS Apple Silicon/Intel (DMG), with checksums and a draft release. Review and publish it. Builds are unsigned; only Windows has been tested locally with the native webview.

**Check for updates** requests the latest published GitHub release only when clicked. It sends no session data; downloading and installing remain manual.

[Export format](docs/SESSIONPACK.md) · [GPL-3.0-only](LICENSE)
