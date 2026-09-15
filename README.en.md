# Pi Desktop

[中文](README.md) · [Development notes](docs/development.md)

A desktop workspace for Pi coding-agent sessions: browse projects and conversations, follow parent/child agents, locate running terminals and continue a session through your local Pi installation.

## Try a local demo

```bash
git clone https://github.com/roshameow/pi-session-viewer.git
cd pi-session-viewer
npm ci
npm run demo
```

Open `/demo.html`. The demo renders the actual sidebar and conversation components with synthetic sessions. It never reads your session directory or calls a model. Requires Node.js 22+.

## Desktop setup

Source builds are available; there is no verified official binary release yet. macOS is the primary development platform. Linux and Windows have partial code paths but are not claimed as fully tested desktop targets.

Install Node.js 22+, Rust 1.88+, [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) and Pi, then run:

```bash
npm ci
npm run tauri dev
```

Install RMUX separately to use persistent terminal features. `npm run build:unsigned` builds a local macOS app without requiring the maintainer's named signing certificate; it is not a notarized distribution. See [development notes](docs/development.md) for stable signing and accessibility permissions.

Features include project grouping, message/tool rendering, nested subagents, terminal status, local Pi continuation, configuration browsing and HTML export.

[Changelog](CHANGELOG.md) · [Issues](https://github.com/roshameow/pi-session-viewer/issues) · [MIT](LICENSE)
