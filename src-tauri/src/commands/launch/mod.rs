mod args;
mod assets;
mod classpath;
mod manifest;

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use log::info;
use tauri::{Emitter, State};
use yaminabe_launcher_shared::datatypes::{InstanceMeta, LaunchMode, ModLoader};
use yaminabe_launcher_shared::error::Error;
use yaminabe_launcher_shared::ipc::LogLine;
use crate::{assets_dir, libraries_dir, runtimes_dir, versions_dir, AppState};
use crate::commands::auth::{
    hydrate_account, persist_account, refresh_account_tokens, MinecraftAccount,
    MinecraftAccountRecord,
};
use crate::commands::instance::find_instance_dir;
use crate::commands::java::download_java_runtime;
use crate::install_task::version_manifest_path;

use self::args::{
    LaunchVars, collect_default_jvm, dedup_preserve_order, process_args, substitute_vars,
};
use self::assets::download_assets;
use self::classpath::{build_classpath, extract_natives, find_main_class_jar, jar_contains_class};
use self::manifest::{load_manifest, merge_manifest};

#[tauri::command]
pub async fn launch_instance(
    app_handle: tauri::AppHandle,
    instance_id: String,
    mc_version: String,
    // `mod_loader` is no longer needed by the backend (instance.json's
    // version_id is the source of truth) but is preserved in the IPC signature
    // for frontend compatibility. Prefix-underscore to silence the unused warning.
    _mod_loader: ModLoader,
    // Optional so existing call sites that omit it keep working; None and
    // Some(Online) are treated identically.
    launch_mode: Option<LaunchMode>,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    let launch_mode = launch_mode.unwrap_or(LaunchMode::Online);
    let instance_location = {
        let install_dir = state.settings.read().unwrap().instance_install_dir.clone();
        find_instance_dir(Path::new(&install_dir), &instance_id)?
            .to_string_lossy()
            .into_owned()
    };
    let instance_meta: Option<InstanceMeta> = std::fs::read_to_string(
        PathBuf::from(&instance_location).join("instance.json")
    ).ok().and_then(|s| serde_json::from_str(&s).ok());

    let (instance_jre_path, ram_mb, window_width, window_height) = {
        let s = state.settings.read().unwrap();
        let jre = instance_meta.as_ref()
            .and_then(|m| if m.jre_path.is_empty() { None } else { Some(m.jre_path.clone()) });
        let ram = instance_meta.as_ref().map(|m| if m.ram_mb > 0 { m.ram_mb } else { s.memory_mb }).unwrap_or(s.memory_mb);
        let width = instance_meta.as_ref().map(|m| if m.window_width > 0 { m.window_width } else { s.window_width }).unwrap_or(s.window_width);
        let height = instance_meta.as_ref().map(|m| if m.window_height > 0 { m.window_height } else { s.window_height }).unwrap_or(s.window_height);
        (jre, ram, width, height)
    };

    // Clone the selected record out under the mutex; secrets are pulled from
    // the OS keyring after the macros are defined so a missing entry can fail
    // through `fail!` with a friendly message.
    let auth_record: Option<MinecraftAccountRecord> = if launch_mode == LaunchMode::Offline {
        None
    } else {
        let store = state.account_store.lock().unwrap();
        store.selected.as_ref().and_then(|uuid| {
            store.accounts.iter().find(|a| &a.uuid == uuid).cloned()
        })
    };

    for dir in [versions_dir(), libraries_dir()] {
        std::fs::create_dir_all(dir)?;
    }

    macro_rules! log {
        ($line:expr) => {
            app_handle.emit("instance-log", LogLine {
                instance_id: instance_id.clone(), line: $line.to_string(), done: false, error: None,
            }).ok();
        };
    }
    macro_rules! done {
        ($line:expr) => {
            app_handle.emit("instance-log", LogLine {
                instance_id: instance_id.clone(), line: $line.to_string(), done: true, error: None,
            }).ok();
        };
    }
    macro_rules! fail {
        ($e:expr) => {{
            let msg = $e.to_string();
            app_handle.emit("instance-log", LogLine {
                instance_id: instance_id.clone(), line: msg.clone(), done: true, error: Some(msg),
            }).ok();
            return Err($e);
        }};
    }

    // Remember this as the most recently launched instance so the navbar's
    // Instant-Play button can relaunch it across sessions. A write failure is
    // non-fatal — it only costs the convenience shortcut.
    {
        let mut s = state.settings.write().unwrap();
        s.last_played_instance_id = instance_id.clone();
        match serde_json::to_string_pretty(&*s) {
            Ok(json) => {
                if let Err(e) = std::fs::write(crate::settings_path(), json) {
                    log::warn!("failed to persist last_played_instance_id: {e}");
                }
            }
            Err(e) => log::warn!("failed to serialize settings: {e}"),
        }
    }

    // Hydrate the cloned record by reading its secret from the OS keyring.
    // A missing keyring entry means the account was removed externally or
    // the migration never ran — bail out asking the user to re-add it.
    let mut auth_account: Option<MinecraftAccount> = match auth_record {
        Some(record) => Some(match hydrate_account(&record) {
            Ok(a) => a,
            Err(e) => fail!(Error::Auth(format!(
                "Stored credentials for {} are missing or unreadable ({e}). \
                 Re-add the account from Settings → Accounts.",
                record.username
            ))),
        }),
        None => None,
    };

    // Refresh the MC token if it's near expiry; 300 s headroom covers the
    // library/asset prefetch that follows.
    if let Some(account) = auth_account.as_mut() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if account.mc_access_token.is_empty() || account.expires_at - now < 300 {
            log!(format!("Refreshing Microsoft credentials for {}...", account.username));
            if let Err(e) = refresh_account_tokens(&state.http_client, account).await {
                fail!(Error::Auth(format!(
                    "Could not refresh credentials for {}: {e}. \
                     Re-add the account from Settings → Accounts.",
                    account.username
                )));
            }
            // Persist refreshed tokens to both the keyring secret and the
            // records file; failure is non-fatal — `account` still holds
            // valid tokens for this launch.
            let mut store = state.account_store.lock().unwrap();
            if let Err(e) = persist_account(&mut store, account) {
                log!(format!("warning: failed to persist refreshed tokens: {e}"));
            }
        }
        log!(format!("Signed in as {}.", account.username));
    } else if launch_mode == LaunchMode::Offline {
        log!("Offline mode selected — launching as OfflinePlayer.");
    } else {
        log!("No Microsoft account selected — launching as OfflinePlayer.");
    }

    log!("Resolving version JSON...");
    // Source of truth: the version_id captured at install time and recorded
    // in instance.json. Pre-refactor instances with no version_id field must
    // be re-created — we don't attempt to guess the on-disk folder name.
    let version_id = match instance_meta.as_ref().map(|m| m.version_id.clone()) {
        Some(id) if !id.is_empty() => id,
        _ => fail!(Error::Invalid(
            "Instance is missing versionId. Please re-create the instance after the latest refactor.".to_string()
        )),
    };
    let manifest_path = version_manifest_path(&version_id);
    if !manifest_path.exists() {
        fail!(Error::Invalid("The required versions for launch could not be found.".to_string()));
    }
    let child_manifest = match load_manifest(&version_id) {
        Ok(m) => m,
        Err(e) => fail!(e),
    };
    let has_parent_manifest = child_manifest.inherits_from.is_some();
    let manifest = match merge_manifest(&version_id) {
        Ok(m) => m,
        Err(e) => fail!(e),
    };

    // jrePath in instance.json wins; otherwise fetch the JRE the manifest
    // recommends. We pick `java.exe` (not `javaw.exe`) so JVM startup errors
    // surface on the captured stdio; CREATE_NO_WINDOW keeps the console hidden.
    let java = if let Some(custom) = instance_jre_path {
        custom
    } else if let Some(component) = manifest.java_version.as_ref().map(|jv| jv.component.clone()) {
        let java_exe = runtimes_dir().join(&component).join("bin").join("java.exe");
        if !java_exe.exists() {
            log!(format!("Recommended runtime '{component}' missing; downloading..."));
            if let Err(e) = download_java_runtime(&component, &state.http_client).await {
                fail!(Error::Invalid(format!("failed to download recommended JRE '{component}': {e}")));
            }
        }
        if !java_exe.exists() {
            fail!(Error::Invalid(format!("java.exe not found after downloading runtime '{component}'")));
        }
        java_exe.to_string_lossy().into_owned()
    } else {
        "java".to_string()
    };

    log!("Downloading libraries...");
    if let Err(e) = crate::install_task::ensure_libraries(&version_id, &state.http_client).await {
        fail!(e);
    }

    let asset_index = match manifest.asset_index.as_ref() {
        Some(idx) => idx,
        None => fail!(Error::Invalid("assetIndex not found in version JSON".into())),
    };
    let asset_index_file_name = format!("{}.json", asset_index.id);
    log!(format!(
        "[AssetManager/INFO]: Loading asset index: {} (id: {})",
        asset_index_file_name, asset_index.id
    ));
    if let Err(e) = download_assets(assets_dir(), asset_index, &state.http_client, |line| {
        log!(line);
    }).await {
        fail!(e);
    }

    let natives_dir = libraries_dir().join("natives");
    log!("Extracting natives...");
    let native_dirs = match extract_natives(&manifest.libraries, libraries_dir(), &natives_dir) {
        Ok(dirs) => dedup_preserve_order(dirs.into_iter().map(|p| p.to_string_lossy().into_owned())),
        Err(e) => {
            log!(format!("Warning: natives: {e}"));
            Vec::new()
        }
    };

    if manifest.main_class.is_empty() {
        fail!(Error::Invalid("mainClass not found in version JSON".into()));
    }

    let entry_libraries = if has_parent_manifest {
        &child_manifest.libraries
    } else {
        &manifest.libraries
    };
    let entry_jar = find_main_class_jar(entry_libraries, libraries_dir(), &manifest.main_class)
        .or_else(|| {
            if has_parent_manifest {
                return None;
            }
            let vanilla_jar = versions_dir().join(&mc_version).join(format!("{mc_version}.jar"));
            (vanilla_jar.exists() && jar_contains_class(&vanilla_jar, &manifest.main_class)).then_some(vanilla_jar)
        });
    let Some(entry_jar) = entry_jar else {
        fail!(Error::Invalid(format!(
            "could not resolve entry jar containing mainClass {}",
            manifest.main_class
        )));
    };
    log!(format!("Entry jar: {}", entry_jar.display()));

    let (classpath, missing_libs) = build_classpath(&manifest.libraries, libraries_dir(), versions_dir(), &mc_version, &version_id);
    for m in &missing_libs {
        log!(format!("Warning: manifest library missing on disk: {m}"));
    }
    if classpath.is_empty() {
        fail!(Error::Invalid("classpath is empty — libraries may not have downloaded correctly".into()));
    }

    let game_dir = PathBuf::from(&instance_location);
    std::fs::create_dir_all(&game_dir)?;
    let game_dir_str = game_dir.to_string_lossy().into_owned();
    let natives_str = if native_dirs.is_empty() {
        natives_dir.to_string_lossy().into_owned()
    } else {
        native_dirs.join(if cfg!(windows) { ";" } else { ":" })
    };
    let asset_index_name = asset_index.id.clone();
    let assets_dir = assets_dir().to_string_lossy().into_owned();

    let res_width = if window_width > 0 { window_width.to_string() } else { String::new() };
    let res_height = if window_height > 0 { window_height.to_string() } else { String::new() };
    let libraries_dir_str = libraries_dir().to_string_lossy().into_owned();
    let classpath_separator = if cfg!(windows) { ";" } else { ":" };

    // When an account is selected its tokens replace the offline defaults so
    // ${auth_*} args expand into real credentials.
    let (auth_player_name, auth_uuid_str, auth_access_token, auth_xuid_str, user_type) = match &auth_account {
        Some(a) => (
            a.username.clone(),
            a.uuid_dashed(),
            a.mc_access_token.clone(),
            if a.xuid.is_empty() { "0".to_string() } else { a.xuid.clone() },
            "msa",
        ),
        None => (
            "OfflinePlayer".to_string(),
            "00000000-0000-0000-0000-000000000000".to_string(),
            "0".to_string(),
            "0".to_string(),
            "offline",
        ),
    };

    let vars = LaunchVars {
        natives_directory: &natives_str,
        classpath: &classpath,
        classpath_separator,
        library_directory: &libraries_dir_str,
        launcher_name: "yaminabe",
        launcher_version: "0.1.0",
        auth_player_name: &auth_player_name,
        version_name: &mc_version,
        game_directory: &game_dir_str,
        assets_root: &assets_dir,
        assets_index_name: &asset_index_name,
        auth_uuid: &auth_uuid_str,
        auth_access_token: &auth_access_token,
        user_type,
        user_properties: "{}",
        version_type: "release",
        clientid: "0",
        auth_xuid: &auth_xuid_str,
        resolution_width: &res_width,
        resolution_height: &res_height,
    };

    // Gated on debug builds so user-facing release logs don't leak the
    // local Java path, instance directory, or main class.
    if cfg!(debug_assertions) {
        log!(format!("Java: {java}"));
        log!(format!("Main class: {}", manifest.main_class));
        log!(format!("Game dir: {game_dir_str}"));
    }

    // Build the full argument list up front so we can log it verbatim before
    // spawning — useful for diagnosing crashes where Java exits before printing.
    // Layout: [JVM args] [main-class | -jar <app.jar>] [game args].
    let mut launch_args: Vec<String> = Vec::new();
    if let Some(args) = &manifest.arguments {
        let mut jvm_section: Vec<String> = Vec::new();
        let default_jvm = collect_default_jvm(&args.default_user_jvm);
        if default_jvm.is_empty() {
            jvm_section.push("-Xms512M".to_string());
            jvm_section.push(format!("-Xmx{ram_mb}M"));
        } else {
            jvm_section.extend(default_jvm);
        }
        jvm_section.extend(process_args(&args.jvm, &vars));
        launch_args.extend(jvm_section);

        // Always launch via mainClass + populated classpath, never `-jar`:
        // Forge's bootstrap launcher needs `${classpath}` and module-path JVM
        // args from the manifest, which `-jar` would ignore.
        launch_args.push(manifest.main_class.clone());
        launch_args.extend(process_args(&args.game, &vars));
    } else {
        launch_args.push("-Xms512M".to_string());
        launch_args.push(format!("-Xmx{ram_mb}M"));
        launch_args.push(format!("-Djava.library.path={natives_str}"));
        launch_args.push("-Dminecraft.launcher.brand=yaminabe".to_string());
        launch_args.push("-cp".to_string());
        launch_args.push(classpath.clone());
        launch_args.push(manifest.main_class.clone());
        if let Some(s) = &manifest.minecraft_arguments {
            launch_args.extend(s.split_whitespace().map(|s| substitute_vars(s, &vars)));
        }
    }

    // Gated to debug builds so the live MC bearer token can't reach a user's
    // log viewer; `cmd.args` below still receives the real values. `{:?}`
    // surfaces hidden whitespace / control chars in dev.
    if cfg!(debug_assertions) {
        log!("Launch command:");
        log!(format!("  {java}"));
        for arg in &launch_args {
            log!(format!("    {arg:?}"));
        }
    }
    log!("Starting process...");

    let mut cmd = tokio::process::Command::new(&java);
    cmd.args(&launch_args);
    cmd.current_dir(&game_dir);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    // CREATE_NO_WINDOW = 0x08000000 — suppress the console window that `java.exe`
    // (a console subsystem binary) would otherwise pop up when launched from a
    // GUI parent. Redirected stdio still works, unlike javaw.
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => fail!(Error::ChildProcess(format!("spawning Java process: {e}"))),
    };
    if let Some(pid) = child.id() {
        state.running_children.lock().unwrap().insert(instance_id.clone(), pid);
        // Frontend gates its Stop control on this event — without it, a click
        // during the preparation phase would race kill_instance against the
        // not-yet-populated `running_children` map.
        app_handle.emit("instance-process-started", &instance_id).ok();
    }

    // Read bytes (not lines) and decode lossily — Java stderr is system-
    // codepage on Windows (e.g. CP932), and `BufReader::lines()` aborts on
    // non-UTF-8, dropping exactly the early-JVM-error output we need most.
    async fn drain(
        mut reader: impl tokio::io::AsyncRead + Unpin,
        app: tauri::AppHandle,
        instance_id: String,
        is_stderr: bool,
    ) {
        use tokio::io::AsyncReadExt;
        let mut buf = [0u8; 4096];
        let mut leftover: Vec<u8> = Vec::new();
        let emit = |app: &tauri::AppHandle, id: &str, bytes: &[u8]| {
            let raw = String::from_utf8_lossy(bytes);
            let trimmed = raw.trim_end_matches(|c| c == '\r' || c == '\n');
            let line = if is_stderr { format!("[STDERR] {trimmed}") } else { trimmed.to_string() };
            app.emit("instance-log", LogLine {
                instance_id: id.to_string(), line, done: false, error: None,
            }).ok();
        };
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    leftover.extend_from_slice(&buf[..n]);
                    while let Some(pos) = leftover.iter().position(|&b| b == b'\n') {
                        let line_bytes: Vec<u8> = leftover.drain(..=pos).collect();
                        emit(&app, &instance_id, &line_bytes);
                    }
                }
            }
        }
        if !leftover.is_empty() {
            emit(&app, &instance_id, &leftover);
        }
    }

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let t1 = tokio::spawn(drain(stdout, app_handle.clone(), instance_id.clone(), false));
    let t2 = tokio::spawn(drain(stderr, app_handle.clone(), instance_id.clone(), true));

    let status = child.wait().await?;
    state.running_children.lock().unwrap().remove(&instance_id);
    t1.await.ok();
    t2.await.ok();

    let exit_code = status.code().unwrap_or(-1);
    log!(format!("Process exited with code {exit_code}."));

    if !status.success() {
        let crash_dir = game_dir.join("crash-reports");
        if let Ok(entries) = std::fs::read_dir(&crash_dir) {
            let mut reports: Vec<std::fs::DirEntry> = entries
                .flatten()
                .filter(|e| e.path().extension().map_or(false, |x| x == "txt"))
                .collect();
            reports.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
            if let Some(latest) = reports.last() {
                if let Ok(content) = std::fs::read_to_string(latest.path()) {
                    log!("─── Crash Report ──────────────────────────────");
                    for line in content.lines() {
                        log!(line.to_string());
                    }
                }
            }
        }
    }

    done!(format!("Done (exit code {exit_code})"));
    info!("Instance {instance_id} exited: {status}");
    Ok(())
}

/// Terminate the Java process tree for a running instance. We shell out to
/// `taskkill /F /T /PID <pid>` rather than calling `tokio::Child::kill` because
/// the `Child` is owned by `launch_instance` (which holds it across an `await`
/// on `wait()`) and forwarding ownership would require restructuring the entire
/// drain/wait pipeline. `/T` walks the descendant tree so child JVMs/launchers
/// spawned by the game (e.g. crash reporters) get cleaned up too.
#[tauri::command]
pub async fn kill_instance(
    instance_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), Error> {
    let pid = state.running_children.lock().unwrap().get(&instance_id).copied();
    let Some(pid) = pid else {
        return Err(Error::NotExists(format!("running instance '{instance_id}'")));
    };

    #[cfg(windows)]
    {
        let status = tokio::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .creation_flags(0x08000000)
            .status().await
            .map_err(|e| Error::ChildProcess(format!("invoking taskkill for pid {pid}: {e}")))?;
        if !status.success() {
            return Err(Error::ChildProcess(format!("taskkill for pid {pid} exited with {status}")));
        }
    }
    #[cfg(not(windows))]
    {
        let status = tokio::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status().await
            .map_err(|e| Error::ChildProcess(format!("invoking kill for pid {pid}: {e}")))?;
        if !status.success() {
            return Err(Error::ChildProcess(format!("kill for pid {pid} exited with {status}")));
        }
    }
    Ok(())
}
