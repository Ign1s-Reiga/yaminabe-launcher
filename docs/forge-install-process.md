# Forge / NeoForge Install & Patch Process

This document describes how Yaminabe Launcher downloads, installs, and patches
Forge and NeoForge clients. It is a map of the existing implementation under
`src-tauri/src/install_task/`, plus the era-specific quirks that the code has to
work around.

## TL;DR

There is no single Forge install format. Forge has changed its installer layout
several times across Minecraft versions, and NeoForge forked from the 1.13+
format with its own small differences. The launcher inspects each installer jar
and routes it to one of two pipelines:

- **V1** (pre-1.13): no patched client jar. A *universal* jar carrying
  binary patches is dropped under `libraries/`, and a LaunchWrapper tweaker
  (`FMLTweaker`) installs a class transformer that applies the patches in-memory
  at runtime against the vanilla client jar.
- **V2** (1.13+ Forge and all NeoForge): the installer ships a `PostProcessors`
  pipeline that *builds* a patched client jar on the user's machine. We
  replicate that pipeline natively in Rust — we do **not** shell out to
  `java -jar <installer> --installClient`.

NeoForge is a fork of Forge that began at **MC 1.20.1** (2023); it inherited the
modern (1.13-origin) processor-based installer format, which is why it always
takes the V2 path.

Vanilla, Fabric, and Quilt have their own separate paths and are only mentioned
here for contrast.

## Entry points

| Loader   | Function                                    | File                       |
|----------|---------------------------------------------|----------------------------|
| Forge    | `ensure_forge`                              | `install_task/mod.rs`      |
| NeoForge | `ensure_neoforge`                           | `install_task/mod.rs`      |
| V1 impl  | `forge_v1::install` / `install_from_parsed` | `install_task/forge_v1.rs` |
| V2 impl  | `forge_v2::install`                         | `install_task/forge_v2.rs` |

Both `ensure_*` functions follow the same shape:

1. Download the installer jar from maven into `temp_dir()`.
2. Read the authoritative `version_id` out of the installer **before** deciding
   whether to skip (so re-installs are idempotent).
3. Run the appropriate install pipeline if the version is not already present.
4. Always run `pre_download_libraries` afterward to backfill anything the
   install step left out.

## Step 1 — Downloading the installer

Forge installers come from `https://maven.minecraftforge.net/`, NeoForge from
`https://maven.neoforged.net/releases/`. Both use `download_from_maven`
(`http_utils.rs`) with a classifier of `installer`.

### Maven version naming is NOT formulaic (important)

The Forge maven *version* (the `<ver>` in `net.minecraftforge:forge:<ver>` and
in `forge/<ver>/forge-<ver>-installer.jar`) is always `{mc}-{loader_version}`,
but the portion *after* `{mc}-` varies per era and is carried inside
`loader_version` itself. Do **not** try to strip or re-append a `-{mc}` suffix
with a single rule. Concrete examples:

- `1.6.1-8.9.0.775` — no MC suffix on the build at all.
- `1.7.2-10.12.2.1161-mc172` — compacted `-mc172`, not `-1.7.2`.
- `1.7.10-10.13.4.1614-1.7.10` — full MC version repeated.
- `1.10-12.18.0.2000-1.10.0` — *expanded* MC version (input was `1.10`).

`forge_maven_version()` therefore just does `format!("{mc}-{loader_version}")`
and treats everything past `{mc}-` as opaque. The exact directory/file name must
come from the source of truth (the version-list layer / maven_metadata.xml),
never from pattern-matching. See `ensure_forge` and the doc comment on
`forge_maven_version`.

NeoForge is simpler: its maven version is a single string like `21.1.79` with no
embedded MC version, so `ensure_neoforge` passes `neoforge-<ver>` straight
through (stripping the `neoforge-` UI prefix).

## Step 2 — Detecting the install type

`detect_install_type` (`mod.rs`) opens the installer jar and inspects entries:

- No `install_profile.json` → **Unsupported** (pre-1.6 jar-mod era; the
  installer download usually 404s before we even get here).
- Has `install_profile.json` **and** `version.json` → **V2**.
- Has `install_profile.json` but **no** `version.json` → **V1**.

The `version_id` is read up front:

- V1: `read_v1_version` parses `install_profile.json` → `versionInfo.id`.
- V2: `read_v2_version` parses `version.json` → `id`.

The installer jar is a zip; all reads go through `installer_archive.rs`
(`open`, `read_entry_bytes`, `read_entry_str`). One-shot reads use the
`read_entry_*` helpers; loops that touch many entries call `open` once and reuse
the archive.

## Step 3a — The V1 pipeline (pre-1.13)

Implemented in `forge_v1.rs`. Pre-1.13 Forge has **no patched client jar**.

`install_profile.json` (V1 schema) carries:

- `install.path` — the maven coordinate of the universal jar
  (`net.minecraftforge:forge:<ver>`).
- `install.filePath` — the entry name of the universal jar *inside* the
  installer.
- `versionInfo` — the full Mojang-style version manifest, nested inline.

`install_from_parsed` does:

1. Write the universal jar to `libraries/<maven path of install.path>`.
   This is the `net.minecraftforge:forge:<ver>` library, **not** a version jar.
2. Write `versionInfo` to `versions/<id>/<id>.json`. The raw JSON text is
   written **verbatim** (not re-serialized through our struct), so unknown
   library fields survive round-tripping.
3. Download each library in `versionInfo.libraries` that has a non-empty `url`
   and is not a classified artifact.

**Ordering matters:** the universal jar is written *before* the manifest,
because callers gate "already installed?" on the manifest's existence — a
present manifest must imply the jar is on disk too.

**How the patch is actually applied:** at launch, the FML LaunchWrapper tweaker
registers a class transformer that applies the binpatches packed inside the
universal jar (`binpatches.pack.lzma`) in-memory, as vanilla classes are loaded,
against the client jar inherited via `inheritsFrom`. Nothing on disk is a
"patched jar" — the patching is a runtime concern.

The tweaker's class name is era-specific: `cpw.mods.fml.common.launcher.FMLTweaker`
on MC 1.6–1.7, and `net.minecraftforge.fml.common.launcher.FMLTweaker` from MC 1.8
onward (FML was repackaged from `cpw.mods.fml` to `net.minecraftforge.fml` at 1.8).

## Step 3b — The V2 pipeline (1.13+ Forge, all NeoForge)

Implemented in `forge_v2.rs::install`. This is the most involved path because it
**produces a patched client jar locally** by running the installer's processor
steps ourselves. We deliberately do not call the installer's `--installClient`
entry point — doing it natively gives us progress reporting, error surfaces, and
independence from installer flag/version quirks.

`install_profile.json` (V2 schema, struct `InstallProfileV2`) carries:

- `minecraft` — the base vanilla MC version.
- `path` — maven coord of the primary jar. **Forge sets it; NeoForge omits the
  field entirely.** It is only actually *needed* when `data` is empty (see the
  empty-processor shortcut below): when `data` is populated, the patched jar's
  coordinate is already available there (the `PATCHED` entry — for Forge it
  resolves to `[net.minecraftforge:forge:<ver>:client]`, for NeoForge to the
  `net.neoforged:neoforge` equivalent), so `path` is redundant — which is why
  NeoForge can leave it out and the field must be treated as optional. Forge's
  value is also not always the bare universal coord (newer Forge uses a
  classifier, e.g. `…:forge:26.1.2-64.0.8:shim`), reinforcing that it should not
  be relied on outside the empty-`data` case.
- `data` — a map of token → sided (`client`/`server`) value used to fill in
  processor args.
- `processors` — the ordered list of JVM invocations that build the patched jar.
- `libraries` — Mojang-style library entries needed by the processors.

`version.json` (separate entry) is the runtime version manifest written to
`versions/<id>/<id>.json`.

### The pipeline, step by step

1. **Read** `install_profile.json` and `version.json` from the installer (one
   `open`, both entries).
2. **Empty-processor shortcut:** if `data` *and* `processors` are both empty
   (e.g. Forge 1.12.2-2864), there is nothing to post-process — the loader uses
   a runtime `FMLTweaker` binpatch like V1. We locate the universal jar from
   `install_profile.libraries` (matching `profile.path`), extract its embedded
   bytes (`maven/<artifact path>`), verify SHA-1, and **delegate to
   `forge_v1::install_from_parsed`**. This is the only consumer of
   `profile.path`.
3. **Write** `version.json` to `versions/<id>/<id>.json`.
4. **Download libraries:** for each entry in `install_profile.libraries`, try
   the copy embedded in the installer under `maven/<artifact path>` first
   (verify SHA-1), and only fall back to the artifact `url` if it is not
   embedded. Entries with neither are skipped (assumed produced by a processor).
5. **Materialize the `data` map** for the client side. Each raw value is
   resolved by `resolve_data_entry`:
   - `[maven:coord]` → absolute path under `libraries/`.
   - `'literal'` → the unquoted literal string.
   - anything else → a resource extracted from the installer jar into a temp
     dir, resolved to its extracted path.
6. **Inject system tokens** into the data map: `SIDE=client`, `MINECRAFT_JAR`
   (the vanilla `versions/<mc>/<mc>.jar`), `MINECRAFT_VERSION`, `ROOT`
   (`bin_dir()`), `INSTALLER` (installer path), `LIBRARY_DIR` (`libraries_dir()`).
7. **Run each processor** in order (`ProcessorSpec`):
   - Skip if `sides` is present and does not include `client`.
   - **Cache hit:** if every declared output already exists and matches its
     expected SHA-1 (`outputs_satisfied`), skip the processor entirely. This is
     what makes re-installs cheap and idempotent.
   - Resolve the processor jar (`maven_coord_to_path`) and read its
     `Main-Class` from `META-INF/MANIFEST.MF` (`read_main_class`, which unfolds
     manifest line-continuations before scanning).
   - Build the classpath: the processor jar plus each `classpath` dependency,
     joined with the OS-specific separator (`;` on Windows, `:` elsewhere). A
     missing dependency is a hard error.
   - Resolve each arg via `resolve_arg`: `[maven:coord]` → libraries path, else
     `{TOKEN}` substitution against the data map (`replace_tokens`).
   - Spawn `java -cp <classpath> <Main-Class> <args...>`. On Windows the
     `CREATE_NO_WINDOW` flag (`0x08000000`) suppresses a console popup.
   - **Verify outputs:** every declared output must exist and match its expected
     SHA-1; on mismatch the file is deleted and the install fails. Outputs are
     never silently accepted.

One JVM is spawned per processor (commonly 4–7 per client install; the client
side skips any `["server"]`-only steps). In-process
`URLClassLoader`-style loading is not feasible from Rust, so we shell out to the
system `java` on PATH — the caller is responsible for having selected the right
JRE.

## Step 4 — Library pre-download / backfill

After either pipeline, `ensure_*` calls a `pre_download_libraries`:

- `forge_v2::pre_download_libraries` walks `versions/<id>/<id>.json` and
  downloads every library with a non-empty `downloads.artifact.url`. Entries
  with an empty `url` (notably the patched `forge-…:client` jar, produced
  locally by the processors) are skipped.
- The shared `install_task::ensure_libraries` (run again before launch) handles
  both manifest schemas — modern `downloads.artifact` entries and legacy bare
  maven-coordinate entries (whose jar lives at `<url base>/<maven path>`,
  defaulting to `https://libraries.minecraft.net/`). It skips natives-bearing
  entries (those classifier jars are unpacked by `extract_natives`, not put on
  the classpath) and anything already on disk, so it is safe to re-run.

`maven_coord_to_path` (`mod.rs`) converts
`group:artifact:version[:classifier][@ext]` into the relative `libraries/` path,
leniently (missing parts become empty segments rather than erroring).

## Launch-time: resolving the patched/inherited manifest

Forge/NeoForge version manifests use `inheritsFrom` to pull in the vanilla base.
`commands/launch/manifest.rs::merge_manifest` resolves this one level deep:

- Single-value fields (`assetIndex`, `minecraftArguments`, `javaVersion`,
  `mainClass`): child wins, parent fills gaps.
- `arguments.{game,jvm,…}`: the standard Mojang arrays are `game` and `jvm`;
  any additional inner arrays the struct models (e.g. a `default_user_jvm`
  extension) are concatenated the same way, parent-first, so child entries can
  override by appearing later.
- `libraries`: child entries kept, parent entries appended only when their name
  is new (dedup by name).

This is why the V1 path can write the version manifest verbatim and still launch
correctly — the vanilla client jar, asset index, and base libraries are all
inherited at merge time.

## Known quirks & gotchas

### NeoForge omits `install_profile.path`

NeoForge does not emit the top-level `path` field at all (it is *absent*, not
`null`). If `InstallProfileV2.path` is typed as a plain `String`, deserialization
fails and the NeoForge install aborts before it starts. This is safe to fix
because `path` is only ever read in the empty-processor shortcut (Step 3b.2),
which NeoForge never reaches — it always has processors, and a populated `data`
map already carries the patched-jar coordinate (`PATCHED`). The fix is to make
the field optional (`Option<String>` with `#[serde(default)]`) and only require
it — erroring if absent — inside the shortcut branch.

### Pre-1.6 jar-mod era is unsupported

Those installers have no `install_profile.json`; `detect_install_type` returns
`Unsupported`. In practice the maven download 404s first.

### NoProfile (pre-1.6 FML bootstrap) blocker

Even with a correct patched client jar, jar-mod-era Forge cannot launch cleanly:
`cpw.mods.fml.relauncher.RelaunchLibraryManager` runs inside the JVM at boot and
tries to download FML bootstrap libs from the long-dead
`http://files.minecraftforge.net/fmllibs/`. FML then checksums the HTML 404
body and aborts. FML *does* honor `<game_directory>/lib/<filename>` if present
with the expected SHA-1, so supporting this era would require a per-Forge-version
table of (filename, sha1, maven coord) seeded into `<instance>/lib/` before
launch. The lib lists are version-specific (1.4.x vs 1.5.x differ). This path
was removed pending that work; do not assume it exists.

### Empty `data` + `processors` is a real, valid V2 shape

Some 1.12.2 builds ship a V2 installer whose post-processing is a no-op. The
shortcut in Step 3b.2 exists specifically for them; without it the install would
produce no patched jar and fail at launch.

### Forge maven version naming

Repeated for emphasis: never derive Forge maven directory/file names with a
formulaic `-{mc}` strip/append. See Step 1.

## File reference

```
src-tauri/src/install_task/
├── mod.rs                   # ensure_forge / ensure_neoforge, type detection,
│                            # version-id reads, maven_coord_to_path, ensure_libraries
├── forge_v1.rs              # pre-1.13: universal jar + runtime FMLTweaker binpatch
├── forge_v2.rs              # 1.13+/NeoForge: native PostProcessors pipeline
├── installer_archive.rs     # zip read helpers for installer jars
└── vanilla.rs / fabric_like.rs  # other loaders (contrast only)

src-tauri/src/commands/launch/
└── manifest.rs              # merge_manifest (inheritsFrom resolution)
```
