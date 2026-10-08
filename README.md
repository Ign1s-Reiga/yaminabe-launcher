# Yaminabe Launcher

> A Minecraft modpack launcher built around the spirit of *yaminabe* — throw a
> jumble of mods into the pot and see what comes out.

**Yaminabe Launcher** is a desktop application for creating, managing, and
running Minecraft modpack instances. *Yaminabe* (闇鍋, "dark hot pot") is a
Japanese party game in which everyone drops a random ingredient into a shared
pot in the dark — nobody knows what the result will taste like. The launcher
brings that same sense of surprise to modpacks: its headline goal is to assemble
modpacks from a random mix of mods.

Today it is a fully functional instance launcher — the foundation the
random-modpack feature is built on. It is a desktop app powered by
[Tauri 2](https://tauri.app/) with a Rust backend and a
[Leptos](https://leptos.dev/) (WebAssembly, client-side) frontend.

## Features

### Instances
- Create and manage multiple instances through a guided three-step wizard
  (creation method → name, Minecraft version & category → mod loader & version).
- Organize the library with category tabs.
- Per-instance settings: a dedicated Java runtime, memory allocation, extra JVM
  arguments, game window size, and a description.

### Mod loaders
- Vanilla, **Forge**, **NeoForge**, **Fabric**, and **Quilt**, installed
  automatically for the chosen Minecraft version (including pre-1.13 Forge
  runtime binpatching and the modern 1.13+ Forge/NeoForge post-processor
  pipeline).

### Launching
- One-click launch with automatic resolution of the version manifest,
  libraries, and asset index, downloading the recommended Mojang Java runtime
  when one isn't already present.
- Live, auto-tailing log viewer that surfaces stdout/stderr and captures crash
  reports when the game exits abnormally.
- **Run several instances at once** — different instances launch concurrently
  and appear in a slide-out *Running* sidebar where you can jump to each one's
  live logs, stop it, or relaunch it. Launching the same instance twice is
  prevented.
- An **Instant-Play** button in the navigation bar relaunches your most recent
  instance, with an online/offline toggle.

### Accounts & play modes
- Sign in with a Microsoft account via a QR code / device code flow.
- **Online** play uses the selected account; **Offline** play skips sign-in.
- Account credentials are kept in the operating system's keyring.

### Library management
- From an instance you can play (online or offline), open its settings, open its
  folders (instance root, `config`, `mods`, `resourcepacks`, `saves`) in the
  system file manager, or delete it with a confirmation step. Launching and
  deletion are mutually exclusive, so an instance can't be removed mid-launch.

### CurseForge
- Search for modpacks on CurseForge and install them directly into your library.

## Roadmap

The defining *yaminabe* features are still simmering:

- [ ] Generate a modpack from a random assortment of mods.
- [ ] Configure how many mods are thrown into a generated pack.
- [ ] Filter the mod pool by category (e.g. API & Library, Technology, Magic).

## Installation

Work in progress...

## Releases & auto-update

The launcher updates itself from this repository's **GitHub Releases**.

**How it works.** Pushing a version tag such as `v0.6.0` runs
`.github/workflows/release.yml`, which builds the Windows installer, signs it for
the updater, and attaches it to a **draft** release together with a
`latest.json` manifest. The release's body is GitHub's generated notes: the pull
requests merged since the previous version's tag. What the launcher itself shows
is that version's section of `CHANGELOG.md`, the same text the Home page shows,
which goes into `latest.json`. The workflow refuses a tag that does not match the
version in `Cargo.toml` and `tauri.conf.json`, or a version with no section in
`CHANGELOG.md`.

Installed launchers read `releases/latest/download/latest.json` once per launch,
compare its version with their own, and verify the installer's signature before
installing, so an update reaches them only once you **publish** the draft.
`Settings → Software Update` shows the running version, checks on request, lists
the new version's changes, and installs it: the installer closes the launcher
and starts the new version. The Settings button in the navigation bar shows a
dot while an update is waiting. Development builds do not check by themselves.

**One-time setup.** Updates are signed with a minisign key that must not live in
the repository. Generate one:

```
cargo tauri signer generate -w ~/.tauri/yaminabe-launcher.key
```

and add these repository secrets:

| Secret | Value |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | Contents of the private key file |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The passphrase you chose |
| `YAMINABE_AZURE_CLIENT_ID` | The Azure app's client id, compiled in for Microsoft sign-in |

The matching **public** key lives in `src-tauri/tauri.conf.json` under
`plugins.updater.pubkey`. The two must stay paired: replacing the key means
installed launchers can no longer verify updates and have to be reinstalled by
hand. Keep a backup of the private key.

Only a tagged release signs. Local builds and CI build the installer without
updater artifacts, so they need no key.

## Continuous integration

`.github/workflows/ci.yml` runs on every pull request and on pushes to `main`:

- **Test Rust backend:** builds the UI with Trunk and runs the backend and shared
  tests.
- **Build installer:** only when a change can reach packaging (the bundle
  config, icons, capabilities, the dependency set, or the workflows), keeping the
  installer as an artifact for 7 days.

The UI depends on [bamboo-css](https://github.com/Ign1s-Reiga/bamboo-css) by
path, so the workflows check it out beside the launcher at a commit pinned in
`.github/actions/setup/action.yml`. Bump that commit when bamboo-css changes.

## License

This software is distributed under the MIT License.
See the [LICENSE](LICENSE) file for details.