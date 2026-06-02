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

## License

This software is distributed under the MIT License.
See the [LICENSE](LICENSE) file for details.