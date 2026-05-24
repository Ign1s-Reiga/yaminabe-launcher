# CLAUDE.md

This file provides STRICT guidance to Claude Code when working with this repository.
Follow these rules to minimize unnecessary context usage and avoid scanning the entire project.

## Architecture Overview

**Yaminabe Launcher** is a Tauri 2.x desktop app for managing game modpacks. It uses a dual-crate workspace:

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
src-ui/src/
├── app.rs                   # Root App component; owns navigation signal (current_nav)
├── ipc.rs                   # Manage IPC functions
├── curseforge.rs            # IPC funcitons utilities for CurseForge API
├── pages/
│   ├── home.rs              # Home Page (`/`)
│   ├── instance_detail.rs   #
│   ├── library.rs           # Library Page (`/library`)
│   ├── play.rs              # Play Page (`/library/:id/play`)
│   ├── search.rs            # Search Page (`/search`)
│   └── settings.rs          # Settings Page (`/settings`)
├── components.rs            # Component module declarations
└── components/
    ├── ui.rs                # Reusable UI primitives
    ├── ui/                  # Atomic UI components (Button, Modal, Input, TabBar)
    ├── create_modal.rs      # Modal used to create new instances
    ├── install_sidebar.rs   # Sidebar that shows install progress
    └── instance_card.rs

src-tauri/src/
├── lib.rs            # Initialize Tauri backend & Register Tauri IPC functions
└── commands/
    ├── curseforge.rs # Commands for CurseForge API
    ├── instance.rs   # Commands for Instance management
    ├── launch.rs     # Commands for Launch Instance
    └── settings.rs   # Commands for Settings management
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

### Formatting Prefrences
- **No Vertical Alignment**: Do not align values or assignments vertically. Avoid adding extra spaces before `=` or `:` to match the positioning of other lines.
- **Single Space Only**: Use only a single space around operators and after delimiters.

### Tauri IPC Pattern

```rust
// Backend (src-tauri/src/lib.rs)
#[tauri::command]
async fn my_command(arg: &str) -> String { ... }

// Frontend (src-ui — via wasm-bindgen)
invoke("my_command", JsValue::from_serde(&args).unwrap()).await
```
