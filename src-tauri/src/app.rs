//! Команди для інтерфейсу. Тонкий шар над ядром: стан застосунку, вікна, події.
use crate::accounts::{self, Account, AccountView};
use crate::content::{self, Kind};
use crate::ctx::Ctx;
use crate::error::{err, Error, Result};
use crate::instances::{self, Instance, NewInstance};
use crate::launch::{self, Auth, Options};
use crate::logparse::LogLine;
use crate::mojang::{self, install, java};
use crate::paths::Paths;
use crate::settings::{self, Settings};
use crate::{loaders, modrinth, news, servers, skins, tasks, util};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;

pub struct AppState {
    pub settings: RwLock<Settings>,
    pub http: reqwest::Client,
    pub accounts: tokio::sync::Mutex<Vec<Account>>,
    pub app: AppHandle,
    preparing: Mutex<HashSet<String>>,
}

type S<'a> = State<'a, Arc<AppState>>;

impl AppState {
    pub fn new(app: AppHandle) -> AppState {
        AppState {
            settings: RwLock::new(Settings::load()),
            http: crate::net::client(),
            accounts: tokio::sync::Mutex::new(accounts::load()),
            app,
            preparing: Mutex::new(HashSet::new()),
        }
    }

    pub fn ctx(&self) -> Ctx {
        let s = self.settings.read().unwrap();
        let app = self.app.clone();
        Ctx {
            paths: Paths::new(s.data_root()),
            http: self.http.clone(),
            emit: Arc::new(move |ev, payload| {
                let _ = app.emit(ev, payload);
            }),
            concurrency: s.concurrency as usize,
        }
    }

    fn settings(&self) -> Settings {
        self.settings.read().unwrap().clone()
    }

    fn update_settings(&self, f: impl FnOnce(&mut Settings)) -> Result<()> {
        let mut s = self.settings.write().unwrap();
        f(&mut s);
        s.save()
    }

    /// Дозволяє показувати в інтерфейсі файли з теки даних (іконки, скріншоти) через asset://.
    pub fn allow_assets(&self) {
        let root = self.ctx().paths.root;
        let _ = self.app.asset_protocol_scope().allow_directory(&root, true);
    }
}

/// Старт: підхопити ігри, що працюють з минулого сеансу, і прибрати тимчасові файли.
pub fn startup(state: &AppState) {
    let ctx = state.ctx();
    tauri::async_runtime::spawn(async move {
        let p = ctx.paths.clone();
        let _ = std::fs::remove_dir_all(p.temp());
        for inst in instances::list(&p) {
            launch::adopt(&ctx, &inst.id, &p.instance_dir(&inst.id));
        }
        // Нативні бібліотеки запущених ігор зайняті — їх не чіпаємо.
        let running: Vec<String> = launch::running().iter().map(|g| format!("{}-", g.instance_id)).collect();
        for e in std::fs::read_dir(p.natives_root()).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !running.iter().any(|r| name.starts_with(r)) {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    });
}

// ---------------------------------------------------------------- загальне

#[tauri::command]
pub async fn app_info(state: S<'_>) -> Result<Value> {
    let ctx = state.ctx();
    Ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "dataDir": ctx.paths.root,
        "configDir": Paths::config_home(),
        "totalMemoryMb": settings::total_memory_mb(),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    }))
}

#[tauri::command]
pub async fn open_url(state: S<'_>, url: String) -> Result<()> {
    let ok = url::Url::parse(&url).is_ok_and(|u| matches!(u.scheme(), "https" | "http"));
    if !ok {
        return Err(err("invalid", "дозволені лише посилання http(s)"));
    }
    state.app.opener().open_url(url, None::<&str>).map_err(|e| err("open", e.to_string()))
}

/// Відкриває теку/файл у провіднику — лише в межах даних лаунчера.
#[tauri::command]
pub async fn open_path(state: S<'_>, path: String, reveal: Option<bool>) -> Result<()> {
    let p = PathBuf::from(&path);
    let allowed = state.ctx().paths.contains(&p) || p.starts_with(Paths::config_home());
    if !allowed || path.contains("..") {
        return Err(err("invalid", "шлях поза теками лаунчера"));
    }
    if !p.exists() {
        std::fs::create_dir_all(&p)?;
    }
    let r = if reveal.unwrap_or(false) { state.app.opener().reveal_item_in_dir(&p) } else { state.app.opener().open_path(path, None::<&str>) };
    r.map_err(|e| err("open", e.to_string()))
}

#[tauri::command]
pub fn task_cancel(id: u64) {
    tasks::cancel(id);
}

#[tauri::command]
pub async fn settings_get(state: S<'_>) -> Result<Settings> {
    Ok(state.settings())
}

#[tauri::command]
pub async fn settings_set(state: S<'_>, settings: Settings) -> Result<Settings> {
    let mut s = settings;
    s.sanitize();
    if let Some(dir) = &s.data_dir {
        let p = PathBuf::from(dir);
        if !p.is_absolute() {
            return Err(err("invalid", "тека даних має бути повним шляхом"));
        }
        std::fs::create_dir_all(&p)?;
    }
    if state.settings().data_root() != s.data_root() && !launch::running().is_empty() {
        return Err(err("busy", "закрийте гру перед зміною теки даних"));
    }
    s.save()?;
    *state.settings.write().unwrap() = s.clone();
    state.allow_assets();
    Ok(s)
}

#[tauri::command]
pub async fn java_probe(path: String) -> Result<Value> {
    let line = java::probe(&PathBuf::from(&path)).await?;
    Ok(json!({ "line": line, "major": java::parse_major(&line) }))
}

// ---------------------------------------------------------------- версії й модлоадери

#[tauri::command]
pub async fn versions_list(state: S<'_>, refresh: Option<bool>) -> Result<Value> {
    let m = mojang::manifest(&state.ctx(), refresh.unwrap_or(false)).await?;
    let versions: Vec<Value> = m.versions.iter().map(|v| json!({ "id": v.id, "kind": v.kind, "releaseTime": v.release_time })).collect();
    Ok(json!({ "latest": m.latest, "versions": versions }))
}

#[tauri::command]
pub async fn loader_versions(state: S<'_>, loader: String, mc: String) -> Result<Vec<loaders::LoaderVersion>> {
    loaders::versions(&state.ctx(), &loader, &mc).await
}

#[tauri::command]
pub async fn loader_games(state: S<'_>, loader: String) -> Result<Vec<String>> {
    loaders::supported_games(&state.ctx(), &loader).await
}

// ---------------------------------------------------------------- збірки

#[tauri::command]
pub async fn instances_list(state: S<'_>) -> Result<Vec<Instance>> {
    let paths = state.ctx().paths;
    Ok(tokio::task::spawn_blocking(move || instances::list(&paths)).await.unwrap_or_default())
}

#[tauri::command]
pub async fn instance_get(state: S<'_>, id: String) -> Result<Instance> {
    instances::get(&state.ctx().paths, &id)
}

/// Shotam у списку серверів нової збірки (якщо на головній його не вимкнули).
async fn seed_servers(state: &AppState, inst: &Instance) {
    if state.settings().show_featured {
        let ctx = state.ctx();
        servers::seed_servers(&ctx.paths.game_dir(&inst.id), &servers::featured(&ctx).await);
    }
}

#[tauri::command]
pub async fn instance_create(state: S<'_>, spec: NewInstance) -> Result<Instance> {
    let inst = instances::create(&state.ctx().paths, spec)?;
    seed_servers(&state, &inst).await;
    state.app.emit("instances-changed", ()).ok();
    Ok(inst)
}

/// Відрізняє «поле не передано» (None) від «скинути на загальне» (Some(None), JSON null).
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> std::result::Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstancePatch {
    name: Option<String>,
    icon: Option<String>,
    mc_version: Option<String>,
    loader: Option<String>,
    #[serde(default, deserialize_with = "present")]
    loader_version: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    memory_mb: Option<Option<u32>>,
    #[serde(default, deserialize_with = "present")]
    java_path: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    jvm_args: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    width: Option<Option<u32>>,
    #[serde(default, deserialize_with = "present")]
    height: Option<Option<u32>>,
    #[serde(default, deserialize_with = "present")]
    fullscreen: Option<Option<bool>>,
}

#[tauri::command]
pub async fn instance_update(state: S<'_>, id: String, patch: InstancePatch) -> Result<Instance> {
    let paths = state.ctx().paths;
    let mut i = instances::get(&paths, &id)?;
    if let Some(n) = patch.name {
        i.name = instances::validate_name(&n)?;
    }
    if let Some(v) = patch.icon {
        i.icon = v;
    }
    let mut reinstall = false;
    if let Some(v) = patch.mc_version.filter(|v| *v != i.mc_version) {
        install::check_id(&v)?;
        i.mc_version = v;
        reinstall = true;
    }
    if let Some(v) = patch.loader.filter(|v| *v != i.loader) {
        if !instances::LOADERS.contains(&v.as_str()) {
            return Err(err("invalid", "невідомий модлоадер"));
        }
        i.loader = v;
        reinstall = true;
    }
    if let Some(v) = patch.loader_version.filter(|v| *v != i.loader_version) {
        i.loader_version = v;
        reinstall = true;
    }
    if reinstall {
        if launch::get(&id).is_some() {
            return Err(err("busy", "закрийте гру перед зміною версії"));
        }
        i.version_id = (i.loader == "vanilla").then(|| i.mc_version.clone());
        if i.loader == "vanilla" {
            i.loader_version = None;
        }
    }
    if let Some(v) = patch.memory_mb {
        i.memory_mb = v.map(|m| m.clamp(512, settings::total_memory_mb().max(2048) as u32));
    }
    if let Some(v) = patch.java_path {
        i.java_path = v.filter(|s| !s.trim().is_empty());
    }
    if let Some(v) = patch.jvm_args {
        i.jvm_args = v.filter(|s| !s.trim().is_empty());
    }
    if let Some(v) = patch.width {
        i.width = v.map(|w| w.clamp(320, 7680));
    }
    if let Some(v) = patch.height {
        i.height = v.map(|h| h.clamp(240, 4320));
    }
    if let Some(v) = patch.fullscreen {
        i.fullscreen = v;
    }
    instances::save(&paths, &i)?;
    state.app.emit("instances-changed", ()).ok();
    Ok(i)
}

#[tauri::command]
pub async fn instance_set_icon_file(state: S<'_>, id: String, path: String) -> Result<Instance> {
    let paths = state.ctx().paths;
    let mut i = instances::get(&paths, &id)?;
    let bytes = std::fs::read(&path)?;
    if bytes.len() > 4 * 1024 * 1024 || !(bytes.starts_with(b"\x89PNG") || bytes.starts_with(&[0xFF, 0xD8]) || bytes.starts_with(b"RIFF")) {
        return Err(err("invalid_image", "потрібне зображення PNG, JPEG або WebP до 4 МБ"));
    }
    util::write_atomic(&paths.instance_dir(&id).join("icon.png"), &bytes)?;
    i.icon = format!("custom:{}", util::now_ms());
    instances::save(&paths, &i)?;
    state.app.emit("instances-changed", ()).ok();
    Ok(i)
}

#[tauri::command]
pub async fn instance_delete(state: S<'_>, id: String) -> Result<()> {
    if launch::get(&id).is_some() || state.preparing.lock().unwrap().contains(&id) {
        return Err(err("busy", "збірка зараз запущена"));
    }
    let paths = state.ctx().paths;
    tokio::task::spawn_blocking(move || instances::delete(&paths, &id)).await.map_err(|e| err("io", e.to_string()))??;
    state.app.emit("instances-changed", ()).ok();
    Ok(())
}

#[tauri::command]
pub async fn instance_duplicate(state: S<'_>, id: String, suffix: String) -> Result<Instance> {
    let paths = state.ctx().paths;
    let inst = tokio::task::spawn_blocking(move || instances::duplicate(&paths, &id, &suffix)).await.map_err(|e| err("io", e.to_string()))??;
    state.app.emit("instances-changed", ()).ok();
    Ok(inst)
}

#[tauri::command]
pub async fn instance_folder(state: S<'_>, id: String, sub: Option<String>) -> Result<String> {
    let p = content::game_path(&state.ctx().paths, &id, sub.as_deref().unwrap_or(""))?;
    Ok(p.display().to_string())
}

#[derive(Deserialize, Clone)]
pub struct ServerTarget {
    host: String,
    port: Option<u16>,
}

#[tauri::command]
pub async fn instance_launch(state: S<'_>, id: String, server: Option<ServerTarget>, world: Option<String>) -> Result<()> {
    if let Some(w) = &world {
        if w.is_empty() || w.contains(['/', '\\']) || w == ".." {
            return Err(err("invalid", "неприпустима назва світу"));
        }
    }
    launch_instance(state.inner().clone(), id, server.map(|s| (s.host, s.port.unwrap_or(25565))), world).await
}

#[tauri::command]
pub async fn instance_repair(state: S<'_>, id: String) -> Result<()> {
    let ctx = state.ctx();
    let mut inst = instances::get(&ctx.paths, &id)?;
    let task = ctx.task_for(inst.name.clone(), &inst.id);
    let r = async {
        let vid = loaders::ensure(&ctx, &inst.loader, &inst.mc_version, inst.loader_version.as_deref(), None, &task).await?;
        let prep = install::prepare(&ctx, &vid, &task, true).await?;
        java::ensure_runtime(&ctx, &prep.json.java_component(), &task, true).await?;
        inst.version_id = Some(vid);
        instances::save(&ctx.paths, &inst)
    }
    .await;
    task.finish(&r);
    r
}

#[tauri::command]
pub fn instance_kill(id: String) -> bool {
    launch::kill(&id)
}

#[tauri::command]
pub fn game_log(id: String) -> Vec<LogLine> {
    launch::log_of(&id)
}

#[tauri::command]
pub fn games_running() -> Vec<Value> {
    launch::running().iter().map(|g| json!({ "instanceId": g.instance_id, "pid": g.pid, "started": g.started })).collect()
}

async fn selected_account(state: &AppState) -> Result<Account> {
    let sel = state.settings().selected_account;
    let mut list = state.accounts.lock().await;
    let idx = list
        .iter()
        .position(|a| Some(&a.id) == sel.as_ref())
        .or(if list.is_empty() { None } else { Some(0) })
        .ok_or_else(|| err("no_account", "додайте акаунт, щоб грати"))?;
    let client_id = state.settings().ms_client_id;
    let acc = &mut list[idx];
    match accounts::ensure_fresh(&state.http, &client_id, acc).await {
        Ok(true) => accounts::save(&list)?,
        Ok(false) => {}
        // Без мережі граємо зі старим токеном: одиночна гра працює, онлайн-сервери — ні.
        Err(Error::Net(_)) => {}
        Err(e) => return Err(e),
    }
    Ok(list[idx].clone())
}

pub async fn launch_instance(state: Arc<AppState>, id: String, server: Option<(String, u16)>, world: Option<String>) -> Result<()> {
    let ctx = state.ctx();
    let mut inst = instances::get(&ctx.paths, &id)?;
    if launch::get(&id).is_some() {
        return Err(err("already_running", "ця збірка вже запущена"));
    }
    if !state.preparing.lock().unwrap().insert(id.clone()) {
        return Err(err("busy", "збірка вже готується до запуску"));
    }
    let settings = state.settings();
    let task = ctx.task_for(inst.name.clone(), &inst.id);
    let r: Result<()> = async {
        let account = selected_account(&state).await?;
        let vid = loaders::ensure(&ctx, &inst.loader, &inst.mc_version, inst.loader_version.as_deref(), inst.version_id.as_deref(), &task).await?;
        if inst.version_id.as_deref() != Some(&vid) {
            inst.version_id = Some(vid.clone());
            instances::save(&ctx.paths, &inst)?;
        }
        let prep = install::prepare(&ctx, &vid, &task, false).await?;
        let java = match inst.java_path.clone().or(settings.java_path.clone()) {
            Some(p) => java::console_variant(&PathBuf::from(p)),
            None => java::ensure_runtime(&ctx, &prep.json.java_component(), &task, false).await?,
        };
        task.stage("launch", 0, "");
        let game_dir = ctx.paths.game_dir(&id);
        let natives_dir = ctx.paths.natives_root().join(format!("{id}-{}", util::now_ms()));
        install::extract_natives(&prep.natives, &natives_dir)?;
        if let Some(install::LegacyAssets::Resources(pairs)) = &prep.legacy_assets {
            install::copy_assets(pairs, &game_dir.join("resources"), &task)?;
        }
        let ms = account.ms.clone();
        let opts = Options {
            game_dir: game_dir.clone(),
            natives_dir: natives_dir.clone(),
            memory_mb: inst.memory_mb.unwrap_or(settings.memory_mb),
            jvm_args: launch::split_args(inst.jvm_args.as_deref().unwrap_or(&settings.jvm_args)),
            resolution: (inst.width.unwrap_or(settings.width), inst.height.unwrap_or(settings.height)),
            fullscreen: inst.fullscreen.unwrap_or(settings.fullscreen),
            server: server.clone(),
            world: world.clone(),
            auth: Auth {
                name: account.name.clone(),
                uuid: account.uuid.clone(),
                token: ms.as_ref().map(|m| m.access_token.clone()).unwrap_or_else(|| "0".into()),
                user_type: if ms.is_some() { "msa" } else { "legacy" },
                xuid: ms.as_ref().map(|m| m.xuid.clone()).unwrap_or_else(|| "0".into()),
            },
        };
        let args = launch::build_args(&prep, &ctx.paths, &opts);
        let paths = ctx.paths.clone();
        let app = state.app.clone();
        let on_launch = settings.on_launch.clone();
        launch::spawn(&ctx, &id, &java, args, &game_dir, natives_dir, move |exit| {
            if let Ok(mut i) = instances::get(&paths, &exit.instance_id) {
                i.play_time += ((util::now_ms() - exit.started).max(0) / 1000) as u64;
                i.last_played = Some(exit.started);
                let _ = instances::save(&paths, &i);
            }
            let _ = app.emit("instances-changed", ());
            if on_launch == "minimize" || exit.crash.is_some() {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        })?;
        inst.last_played = Some(util::now_ms());
        instances::save(&ctx.paths, &inst)?;
        state.update_settings(|s| s.last_instance = Some(id.clone()))?;
        Ok(())
    }
    .await;
    state.preparing.lock().unwrap().remove(&id);
    task.finish(&r);
    if r.is_ok() {
        state.app.emit("instances-changed", ()).ok();
        if let Some(w) = state.app.get_webview_window("main") {
            match settings.on_launch.as_str() {
                "minimize" => {
                    let _ = w.minimize();
                }
                "close" => {
                    // Гра — окремий процес і працює далі без лаунчера.
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    state.app.exit(0);
                }
                _ => {}
            }
        }
    }
    r
}

// ---------------------------------------------------------------- закріплений сервер

#[tauri::command]
pub async fn featured_get(state: S<'_>) -> Result<servers::Featured> {
    Ok(servers::featured(&state.ctx()).await)
}

#[tauri::command]
pub async fn server_ping(host: String, port: Option<u16>) -> Result<servers::Status> {
    servers::ping(&host, port.unwrap_or(25565)).await
}

/// «Грати на Shotam»: своя збірка під версію сервера з потрібними модами і швидкий вхід.
#[tauri::command]
pub async fn featured_play(state: S<'_>) -> Result<String> {
    // Спершу акаунт — щоб не качати збірку, з якою однаково не можна зайти.
    if state.accounts.lock().await.is_empty() {
        return Err(err("no_account", "додайте акаунт, щоб грати"));
    }
    let ctx = state.ctx();
    let f = servers::featured(&ctx).await;
    let existing = instances::list(&ctx.paths).into_iter().find(|i| i.featured.as_deref() == Some(f.id.as_str()));
    let mut inst = match existing {
        Some(i) => i,
        None => {
            let mut i = instances::create(
                &ctx.paths,
                NewInstance { name: f.name.clone(), mc_version: f.mc_version.clone(), loader: f.loader.clone(), loader_version: None, icon: Some("featured:shotam".into()) },
            )?;
            i.featured = Some(f.id.clone());
            instances::save(&ctx.paths, &i)?;
            i
        }
    };
    // Сервер перейшов на іншу версію — переводимо збірку слідом.
    if inst.mc_version != f.mc_version || inst.loader != f.loader {
        inst.mc_version = f.mc_version.clone();
        inst.loader = f.loader.clone();
        inst.loader_version = None;
        inst.version_id = (f.loader == "vanilla").then(|| f.mc_version.clone());
        instances::save(&ctx.paths, &inst)?;
    }
    if !f.mods.is_empty() && launch::get(&inst.id).is_none() {
        let task = ctx.task_for(f.name.clone(), &inst.id);
        let mods_dir = ctx.paths.game_dir(&inst.id).join("mods");
        for slug in &f.mods {
            // Без модів (голосовий чат) грати однаково можна — помилку не блокуємо.
            if let Ok(p) = modrinth::project(&ctx, slug).await {
                let pid = p["id"].as_str().unwrap_or("").to_string();
                let index = content::Index::load(&ctx.paths, &inst.id);
                let present = index.files_of(&pid).iter().any(|f| mods_dir.join(f).exists() || mods_dir.join(format!("{f}.disabled")).exists());
                if !present || modrinth_outdated(&ctx, &inst, &pid).await {
                    let _ = modrinth::install(&ctx, &inst, &pid, None, &task).await;
                }
            }
        }
        task.finish::<()>(&Ok(()));
    }
    state.app.emit("instances-changed", ()).ok();
    let id = inst.id.clone();
    launch_instance(state.inner().clone(), id.clone(), Some((f.address.clone(), f.port)), None).await?;
    Ok(id)
}

/// Чи підходить встановлена версія мода поточній версії гри збірки.
async fn modrinth_outdated(ctx: &Ctx, inst: &Instance, project_id: &str) -> bool {
    let index = content::Index::load(&ctx.paths, &inst.id);
    let files = index.files_of(project_id);
    let Some(file) = files.first() else { return true };
    let Some(meta) = index.get(Kind::Mods, file) else { return true };
    match modrinth::version(ctx, &meta.version_id).await {
        Ok(v) => !v["game_versions"].as_array().is_some_and(|a| a.iter().any(|g| g == inst.mc_version.as_str())),
        Err(_) => false,
    }
}

// ---------------------------------------------------------------- вміст збірки

#[tauri::command]
pub async fn content_list(state: S<'_>, id: String, kind: String) -> Result<Vec<content::Item>> {
    let kind = Kind::parse(&kind)?;
    let paths = state.ctx().paths;
    Ok(tokio::task::spawn_blocking(move || content::list(&paths, &id, kind)).await.unwrap_or_default())
}

#[tauri::command]
pub async fn content_toggle(state: S<'_>, id: String, kind: String, file: String) -> Result<String> {
    content::toggle(&state.ctx().paths, &id, Kind::parse(&kind)?, &file)
}

#[tauri::command]
pub async fn content_remove(state: S<'_>, id: String, kind: String, file: String) -> Result<()> {
    content::remove(&state.ctx().paths, &id, Kind::parse(&kind)?, &file)
}

#[tauri::command]
pub async fn content_add(state: S<'_>, id: String, kind: String, files: Vec<String>) -> Result<usize> {
    let files: Vec<PathBuf> = files.into_iter().map(PathBuf::from).collect();
    content::add_files(&state.ctx().paths, &id, Kind::parse(&kind)?, &files)
}

/// Дізнається на Modrinth, що за файли лежать у збірці (назви, іконки) — для доданих вручну.
#[tauri::command]
pub async fn content_identify(state: S<'_>, id: String, kind: String) -> Result<Vec<content::Item>> {
    let ctx = state.ctx();
    let kind = Kind::parse(&kind)?;
    let items = content::list(&ctx.paths, &id, kind);
    let unknown: Vec<String> = items.iter().filter(|i| i.project_id.is_none() && !i.is_dir).map(|i| i.file.clone()).collect();
    if unknown.is_empty() {
        return Ok(items);
    }
    let paths = ctx.paths.clone();
    let id2 = id.clone();
    let by_hash = tokio::task::spawn_blocking(move || content::hashes(&paths, &id2, kind, &unknown)).await.unwrap_or_default();
    let found = modrinth::identify(&ctx, &by_hash.keys().cloned().collect::<Vec<_>>()).await?;
    let mut index = content::Index::load(&ctx.paths, &id);
    for (hash, (v, p)) in found {
        if let Some(file) = by_hash.get(&hash) {
            index.set(
                kind,
                file,
                content::Meta {
                    project_id: p["id"].as_str().unwrap_or("").into(),
                    version_id: v["id"].as_str().unwrap_or("").into(),
                    version_number: v["version_number"].as_str().unwrap_or("").into(),
                    title: p["title"].as_str().unwrap_or("").into(),
                    icon_url: p["icon_url"].as_str().map(str::to_string),
                    author: None,
                },
            );
        }
    }
    index.save(&ctx.paths, &id)?;
    Ok(content::list(&ctx.paths, &id, kind))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    file: String,
    version_id: String,
    version_number: String,
    project_id: String,
}

#[tauri::command]
pub async fn content_updates(state: S<'_>, id: String, kind: String) -> Result<Vec<Update>> {
    let ctx = state.ctx();
    let kind = Kind::parse(&kind)?;
    let inst = instances::get(&ctx.paths, &id)?;
    let items = content::list(&ctx.paths, &id, kind);
    let files: Vec<String> = items.iter().filter(|i| !i.is_dir).map(|i| i.file.clone()).collect();
    let paths = ctx.paths.clone();
    let id2 = id.clone();
    let by_hash = tokio::task::spawn_blocking(move || content::hashes(&paths, &id2, kind, &files)).await.unwrap_or_default();
    let latest = modrinth::updates(&ctx, &by_hash.keys().cloned().collect::<Vec<_>>(), &inst, kind).await?;
    let index = content::Index::load(&ctx.paths, &id);
    let mut out = Vec::new();
    for (hash, v) in latest {
        let Some(file) = by_hash.get(&hash) else { continue };
        let new_id = v["id"].as_str().unwrap_or("");
        // Файл уже цієї версії (хеш збігся з одним із файлів нової версії).
        let same = v["files"].as_array().is_some_and(|fs| fs.iter().any(|f| f["hashes"]["sha1"] == hash.as_str()));
        if same || index.get(kind, file).is_some_and(|m| m.version_id == new_id) {
            continue;
        }
        out.push(Update {
            file: file.clone(),
            version_id: new_id.into(),
            version_number: v["version_number"].as_str().unwrap_or("").into(),
            project_id: v["project_id"].as_str().unwrap_or("").into(),
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn content_update(state: S<'_>, id: String, kind: String, file: String, project_id: String, version_id: String) -> Result<()> {
    let ctx = state.ctx();
    let kind = Kind::parse(&kind)?;
    let inst = instances::get(&ctx.paths, &id)?;
    let task = ctx.task_for(inst.name.clone(), &inst.id);
    let r = async {
        let installed = modrinth::install(&ctx, &inst, &project_id, Some(&version_id), &task).await?;
        let disabled = file.ends_with(".disabled");
        if !installed.iter().any(|i| i.file == file.trim_end_matches(".disabled")) {
            let _ = content::remove(&ctx.paths, &id, kind, &file);
        }
        // Вимкнений мод лишається вимкненим і після оновлення.
        if disabled {
            if let Some(new) = installed.iter().find(|i| i.project_id == project_id) {
                content::toggle(&ctx.paths, &id, kind, &new.file)?;
            }
        }
        Ok(())
    }
    .await;
    task.finish(&r);
    r
}

#[tauri::command]
pub async fn worlds_list(state: S<'_>, id: String) -> Result<Vec<content::World>> {
    let paths = state.ctx().paths;
    Ok(tokio::task::spawn_blocking(move || content::worlds(&paths, &id)).await.unwrap_or_default())
}

#[tauri::command]
pub async fn screenshots_list(state: S<'_>, id: String) -> Result<Vec<content::Shot>> {
    Ok(content::screenshots(&state.ctx().paths, &id))
}

/// Видалення світу чи скріншота — у кошик.
#[tauri::command]
pub async fn game_file_trash(state: S<'_>, id: String, rel: String) -> Result<()> {
    if rel.trim().is_empty() {
        return Err(err("invalid", "порожній шлях"));
    }
    let p = content::game_path(&state.ctx().paths, &id, &rel)?;
    trash::delete(&p).map_err(|e| err("io", e.to_string()))
}

// ---------------------------------------------------------------- Modrinth

#[tauri::command]
pub async fn mr_search(state: S<'_>, params: modrinth::SearchParams) -> Result<Value> {
    modrinth::search(&state.ctx(), &params).await
}

#[tauri::command]
pub async fn mr_project(state: S<'_>, id: String) -> Result<Value> {
    modrinth::project(&state.ctx(), &id).await
}

#[tauri::command]
pub async fn mr_versions(state: S<'_>, id: String, loaders: Vec<String>, game_versions: Vec<String>) -> Result<Vec<Value>> {
    modrinth::project_versions(&state.ctx(), &id, &loaders, &game_versions).await
}

#[tauri::command]
pub async fn mr_install(state: S<'_>, instance_id: String, project_id: String, version_id: Option<String>) -> Result<Vec<modrinth::Installed>> {
    let ctx = state.ctx();
    let inst = instances::get(&ctx.paths, &instance_id)?;
    let task = ctx.task_for(inst.name.clone(), &inst.id);
    let r = modrinth::install(&ctx, &inst, &project_id, version_id.as_deref(), &task).await;
    task.finish(&r);
    r
}

#[tauri::command]
pub async fn mr_install_modpack(state: S<'_>, version_id: String, title: String) -> Result<Instance> {
    let ctx = state.ctx();
    let task = ctx.task(title);
    let r = modrinth::install_modpack(&ctx, &version_id, &task).await;
    task.finish(&r);
    if let Ok(inst) = &r {
        seed_servers(&state, inst).await;
    }
    state.app.emit("instances-changed", ()).ok();
    r
}

#[tauri::command]
pub async fn mrpack_import(state: S<'_>, path: String) -> Result<Instance> {
    let ctx = state.ctx();
    let name = PathBuf::from(&path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let task = ctx.task(name);
    let r = modrinth::import_mrpack(&ctx, &PathBuf::from(&path), None, &task).await;
    task.finish(&r);
    state.app.emit("instances-changed", ()).ok();
    r
}

// ---------------------------------------------------------------- акаунти

#[tauri::command]
pub async fn accounts_list(state: S<'_>) -> Result<Value> {
    let list: Vec<AccountView> = state.accounts.lock().await.iter().map(Account::view).collect();
    let sel = state.settings().selected_account;
    let selected = list.iter().find(|a| Some(&a.id) == sel.as_ref()).or(list.first()).map(|a| a.id.clone());
    Ok(json!({ "accounts": list, "selected": selected, "msAvailable": !state.settings().ms_client_id.is_empty() }))
}

async fn upsert(state: &AppState, acc: Account) -> Result<AccountView> {
    let mut list = state.accounts.lock().await;
    list.retain(|a| a.id != acc.id);
    list.push(acc.clone());
    accounts::save(&list)?;
    drop(list);
    state.update_settings(|s| s.selected_account = Some(acc.id.clone()))?;
    state.app.emit("accounts-changed", ()).ok();
    Ok(acc.view())
}

#[tauri::command]
pub async fn account_add_offline(state: S<'_>, name: String) -> Result<AccountView> {
    upsert(&state, accounts::offline(&name)?).await
}

#[tauri::command]
pub async fn account_select(state: S<'_>, id: String) -> Result<()> {
    state.update_settings(|s| s.selected_account = Some(id))?;
    state.app.emit("accounts-changed", ()).ok();
    Ok(())
}

#[tauri::command]
pub async fn account_remove(state: S<'_>, id: String) -> Result<()> {
    let mut list = state.accounts.lock().await;
    list.retain(|a| a.id != id);
    accounts::save(&list)?;
    drop(list);
    if state.settings().selected_account.as_deref() == Some(&id) {
        state.update_settings(|s| s.selected_account = None)?;
    }
    state.app.emit("accounts-changed", ()).ok();
    Ok(())
}

/// Вхід Microsoft у вікні з офіційною сторінкою входу. Код перехоплюється з переадресації.
#[tauri::command]
pub async fn account_login_ms(state: S<'_>) -> Result<AccountView> {
    let client_id = state.settings().ms_client_id;
    if client_id.is_empty() {
        return Err(err("ms_no_client_id", "не вказано Client ID застосунку Azure"));
    }
    let pkce = accounts::authorize_url(&client_id);
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String>>();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let send = {
        let tx = tx.clone();
        move |r: Result<String>| {
            if let Some(tx) = tx.lock().unwrap().take() {
                let _ = tx.send(r);
            }
        }
    };
    if let Some(old) = state.app.get_webview_window("ms-login") {
        let _ = old.close();
    }
    let url: url::Url = pkce.url.parse().map_err(|_| err("invalid", "URL входу"))?;
    let nav_send = send.clone();
    let win = WebviewWindowBuilder::new(&state.app, "ms-login", WebviewUrl::External(url))
        .title("Microsoft")
        .inner_size(520.0, 700.0)
        .resizable(false)
        .center()
        .on_navigation(move |u| {
            if u.as_str().starts_with(accounts::MS_REDIRECT) {
                let q: HashMap<String, String> = u.query_pairs().map(|(k, v)| (k.to_string(), v.to_string())).collect();
                match (q.get("code"), q.get("error")) {
                    (Some(code), _) => nav_send(Ok(code.clone())),
                    (None, Some(e)) if e == "access_denied" => nav_send(Err(Error::Cancelled)),
                    (None, e) => nav_send(Err(err("ms_oauth", format!("{} {}", e.cloned().unwrap_or_default(), q.get("error_description").cloned().unwrap_or_default())))),
                }
                return false;
            }
            true
        })
        .build()
        .map_err(|e| err("window", e.to_string()))?;
    let close_send = send.clone();
    win.on_window_event(move |e| {
        if matches!(e, tauri::WindowEvent::Destroyed) {
            close_send(Err(Error::Cancelled));
        }
    });
    let code = rx.await.map_err(|_| Error::Cancelled)?;
    let _ = win.close();
    let acc = accounts::login_with_code(&state.http, &client_id, &code?, &pkce.verifier).await?;
    upsert(&state, acc).await
}

async fn ms_token(state: &AppState, id: &str) -> Result<String> {
    let client_id = state.settings().ms_client_id;
    let mut list = state.accounts.lock().await;
    let acc = list.iter_mut().find(|a| a.id == id).ok_or_else(|| err("no_account", "акаунт не знайдено"))?;
    if accounts::ensure_fresh(&state.http, &client_id, acc).await? {
        let snapshot = list.clone();
        accounts::save(&snapshot)?;
    }
    let list = &*list;
    list.iter().find(|a| a.id == id).and_then(|a| a.ms.as_ref()).map(|m| m.access_token.clone()).ok_or_else(|| err("invalid", "це не акаунт Microsoft"))
}

/// Скін і плащі акаунта для 3D-перегляду.
#[tauri::command]
pub async fn account_skin(state: S<'_>, id: String) -> Result<Value> {
    let acc = state.accounts.lock().await.iter().find(|a| a.id == id).cloned().ok_or_else(|| err("no_account", "акаунт не знайдено"))?;
    let http = &state.http;
    if acc.kind == "microsoft" {
        let token = ms_token(&state, &id).await?;
        let p = accounts::profile(http, &token).await?;
        let skin = p.skins.iter().find(|s| s.state == "ACTIVE").or(p.skins.first());
        let skin_url = match skin {
            Some(s) => Some(skins::data_url(http, &s.url).await?),
            None => None,
        };
        let mut capes = Vec::new();
        for c in &p.capes {
            capes.push(json!({ "id": c.id, "alias": c.alias, "active": c.state == "ACTIVE", "url": skins::data_url(http, &c.url).await.ok() }));
        }
        return Ok(json!({
            "skin": skin_url,
            "slim": skin.and_then(|s| s.variant.as_deref()).is_some_and(|v| v.eq_ignore_ascii_case("slim")),
            "capes": capes,
            "source": "microsoft",
        }));
    }
    if let Ok(Some((url, slim))) = accounts::skin_by_name(http, &acc.name).await {
        if let Ok(data) = skins::data_url(http, &url).await {
            return Ok(json!({ "skin": data, "slim": slim, "capes": [], "source": "mojang_name" }));
        }
    }
    let paths = state.ctx().paths;
    let local = tokio::task::spawn_blocking(move || skins::default_skin(&paths)).await.ok().flatten();
    Ok(json!({ "skin": local.map(|b| skins::png_url(&b)), "slim": false, "capes": [], "source": "default" }))
}

#[tauri::command]
pub async fn skin_upload(state: S<'_>, id: String, path: String, slim: bool) -> Result<()> {
    let bytes = std::fs::read(&path)?;
    if !bytes.starts_with(b"\x89PNG") || bytes.len() > 1024 * 1024 {
        return Err(err("invalid_skin", "скін має бути файлом PNG до 1 МБ"));
    }
    let token = ms_token(&state, &id).await?;
    accounts::upload_skin(&state.http, &token, bytes, slim).await
}

#[tauri::command]
pub async fn cape_set(state: S<'_>, id: String, cape_id: Option<String>) -> Result<()> {
    let token = ms_token(&state, &id).await?;
    accounts::set_cape(&state.http, &token, cape_id.as_deref()).await
}

// ---------------------------------------------------------------- перехід з інших лаунчерів

#[tauri::command]
pub async fn mcdir_scan(path: Option<String>) -> Result<crate::migrate::Scan> {
    Ok(tokio::task::spawn_blocking(move || crate::migrate::scan(path.as_deref())).await.map_err(|e| err("io", e.to_string()))?)
}

#[tauri::command]
pub async fn mcdir_import(state: S<'_>, path: String, spec: NewInstance, what: crate::migrate::What) -> Result<Instance> {
    let ctx = state.ctx();
    let inst = instances::create(&ctx.paths, spec)?;
    let task = ctx.task_for(inst.name.clone(), &inst.id);
    let dest = ctx.paths.game_dir(&inst.id);
    let task2 = task.clone();
    let r = tokio::task::spawn_blocking(move || crate::migrate::copy_into(&PathBuf::from(path), &dest, &what, &task2))
        .await
        .map_err(|e| err("io", e.to_string()))
        .and_then(|r| r);
    task.finish(&r);
    if let Err(e) = r {
        let _ = std::fs::remove_dir_all(ctx.paths.instance_dir(&inst.id));
        return Err(e);
    }
    seed_servers(&state, &inst).await;
    state.app.emit("instances-changed", ()).ok();
    Ok(inst)
}

// ---------------------------------------------------------------- новини

#[tauri::command]
pub async fn news_list(state: S<'_>) -> Result<Vec<news::Entry>> {
    news::patch_notes(&state.ctx()).await
}

#[tauri::command]
pub async fn news_body(state: S<'_>, path: String) -> Result<String> {
    news::patch_body(&state.ctx(), &path).await
}
