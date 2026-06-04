# CLAUDE.md

This file provides STRICT guidance to Claude Code when working with this repository.
Follow these rules to minimize unnecessary context usage and avoid scanning the entire project.

## Architecture Overview

**Yaminabe Launcher** is a Tauri 2.x desktop app for managing game modpacks. It uses a three-crate workspace:

- **`src-shared/`** — Wire types shared by both ends of the IPC boundary (no Tauri, no Leptos)
- **`src-tauri/`** — Rust backend, Tauri window management, IPC commands (`#[tauri::command]`)
- **`src-ui/`** — Frontend compiled to WebAssembly via Trunk; uses the **Leptos 0.8** reactive framework in CSR mode

### Frontend Stack

| Concern      | Library                                                                            |
|--------------|------------------------------------------------------------------------------------|
| UI framework | Leptos 0.8 (CSR)                                                                   |
| Icons        | phosphor-leptos                                                                    |
| CSS-in-Rust  | `bamboo-css-macro` (local crate at `../../bamboo-css/`), `styled!` macro           |
| Global theme | `src-ui/styles.css` (CSS custom properties, light/dark via `prefers-color-scheme`) |
| Tauri IPC    | `wasm-bindgen` + `wasm-bindgen-futures`                                            |

### Project Structure

```
src-shared/src/
├── lib.rs
├── error.rs                 # `Error` enum + thiserror impls + Serialize for the wire
├── datatypes.rs             # InstanceMeta, AppSettings, ModLoader, ModpackInfo, …
├── ipc.rs                   # Tauri event payloads (LogLine, InstallProgress)
└── version_manifest.rs      # Unified Mojang version JSON: covers vanilla /
                             # Forge / NeoForge / Fabric / Quilt manifests in one
                             # `ClientManifest`. Era-specific fields are Option-
                             # /default-tolerant; skip_serializing_if keeps re-
                             # serialised output close to the original shape so
                             # the V1 install path can round-trip the manifest.

src-tauri/src/
├── lib.rs                   # Tauri init, AppState, IPC handler registration
├── main.rs
├── http_utils.rs            # Shared HTTP+sha1: download_resource,
│                            # fetch_and_verify, verify_sha1, download_from_maven
├── commands/                # `#[tauri::command]` handlers
│   ├── mod.rs
│   ├── instance.rs          # create_instance, get_instances, save_instance_settings
│   ├── java.rs              # Local Java detection + Mojang JRE download
│   ├── minecraft.rs         # Mojang version-manifest-v2 fetch + cache
│   ├── modfile.rs           # Bridge to mod_repo: search, files, install, download
│   ├── settings.rs          # get/save AppSettings, pick_folder, subfolder helpers
│   └── launch/              # launch_instance + kill_instance
│       ├── mod.rs           # Tauri command body + lifecycle + stdio drain
│       ├── manifest.rs      # load_manifest + merge_manifest (inheritsFrom)
│       ├── classpath.rs     # extract_natives, build_classpath, find_main_class_jar
│       ├── args.rs          # LaunchVars, substitute_vars, eval_rules, process_args
│       └── assets.rs        # Asset index sync against resources.download.minecraft.net
├── install_task/            # Per-loader install pipelines (vanilla + loader prep)
│   ├── mod.rs               # ensure_{vanilla,fabric,quilt,forge,neoforge} + ensure_libraries
│   ├── vanilla.rs           # Pre-download vanilla libraries + log4j config
│   ├── fabric_like.rs       # Fabric / Quilt installer + lib prefetch
│   ├── forge_v1.rs          # Pre-1.13 Forge: runtime binpatching (universal jar under libraries/)
│   ├── forge_v2.rs          # 1.13+ Forge / NeoForge: native PostProcessors pipeline
│   └── installer_archive.rs # Zip-read helpers for installer jars (one-shot reads
│                            # use `read_entry_*`; multi-entry loops call `open` directly)
└── mod_repo/                # External mod-repository clients
    ├── mod.rs
    ├── curseforge.rs        # CurseForge API: modpack search/files, modpack install,
    │                        # mod parallel download
    └── modrinth.rs          # Modrinth API: mod download by version id

src-ui/src/
├── main.rs                  # mount_to_body
├── app.rs                   # Router, install-sidebar wiring, navbar
├── ipc.rs                   # `call` / `call_noargs` / `on_event` wrappers around Tauri
├── curseforge.rs            # Frontend wrappers around CurseForge backend commands
├── pages.rs / pages/
│   ├── home.rs              # `/`
│   ├── library.rs           # `/library` — instance grid + category tabs
│   ├── instance_detail.rs   # `/library/:id` — Description / Mods / Settings tabs
│   ├── play.rs              # `/library/:id/play` — runs the launcher, shows logs
│   ├── search.rs            # `/search` — CurseForge modpack search
│   └── settings.rs          # `/settings`
├── components.rs / components/
│   ├── ui.rs / ui/          # Atomic UI primitives (Button, Modal, Input, TabBar,
│   │                        # Segmented, Skeletons)
│   ├── create_modal/        # 3-step "create instance" wizard
│   │   ├── mod.rs           # Shell + `WizardState` (Copy bundle of RwSignals) +
│   │   │                    # step routing + per-loader version prefetch
│   │   ├── step_method.rs   # Step 1 — pick creation method
│   │   ├── step_basics.rs   # Step 2 — name + MC version + category
│   │   └── step_loader.rs   # Step 3 — mod loader + loader version
│   ├── install_modpack_modal.rs
│   ├── install_sidebar.rs   # Slide-in panel showing per-instance install progress
│   ├── instance_card.rs
│   ├── log_viewer.rs        # Dark sticky-tail log box used by `play.rs`. Tail mode
│   │                        # only disengages on *upward* scrolls (downward events
│   │                        # during log bursts are racey); text selection pauses
│   │                        # auto-scroll until a window-level mouseup fires.
│   ├── open_in_file_manager.rs
│   ├── pagination.rs        # Numeric pager with first/last/current ± 1 visible,
│   │                        # ellipsis gaps elsewhere
│   ├── result_card.rs       # CurseForge modpack search-result card
│   └── settings.rs          # SettingsSection / SettingsProp / SaveState scaffolding
└── styles.css               # Global CSS variables + font stacks
```

### CSS Styling Approach

1. **Global variables** — `src-ui/styles.css` defines `--color-*`, `--spacing-*`, and font stacks (Inter, Lexend, IBM Plex Sans JP).
2. **Scoped component styles** — Use `styled!` macro; the `bamboo-css-collector` pre-build hook (configured in `src-ui/Trunk.toml`) collects all styled macros into `src-ui/assets/bundle.css` at build time.
3. **Inline component CSS** — `css! { ... }` macro from `bamboo-css-macro` for ad-hoc styles.

See docs/bamboo-css.md to know how to use it.
Do NOT refactor styling unless explicitly requested.

### JSON Parse Strategy

- Use `serde_json`.
- Do not use `serde_json::Value` as type of parsed JSON.

### Rust Code Rules

- Do not use `let _ = ...` pattern to consume value.
- Inline comments (`//`) should be kept to 2-3 lines or less.
- Inline comments should not be used as separators.
- Documentation comments (`///`) can be any number of lines, up to 10 lines. However, fewer comments are preferable.
  - However, documentation comments (`//!`) for the entire source file should not be written.

### Formatting Prefrences
- **No Vertical Alignment**: Do not align values or assignments vertically. Avoid adding extra spaces before `=` or `:` to match the positioning of other lines.
- **Single Space Only**: Use only a single space around operators and after delimiters.
- **Newline at EOF**: Insert Newline at End of File.

### Tauri IPC Pattern

```rust
// Backend (src-tauri/src/lib.rs)
#[tauri::command]
async fn my_command(arg: &str) -> String { ... }

// Frontend (src-ui — via wasm-bindgen)
invoke("my_command", JsValue::from_serde(&args).unwrap()).await
```

## Git

- Create a dedicated branch for each issue/feature before making any changes; never commit issue work directly to `main`. Work on two issues lives on two separate branches.
  - Name branches `<type>/<branch-name>`, where `<type>` is the Conventional Commits type (`feat`, `fix`, `refactor`, `perf`, `docs`, …) — e.g. `feat/instance-origin`.
- The commit message title should follow Conventional Commits guidelines.
  - Limit commit scopes to `ui`, `launch`, `auth`, `install`, and `shared`. Use the one that best applies.
  - Do not use a scope if none of these apply.
  - Do not apply scopes to the type in the PR title.
- The commit message description should be concise.
- Include only information relevant to the code changes; omit anything else.
- Please use `git` instead of `gh` for basic Git operations (i.e., everything you can do with the `git` command).
