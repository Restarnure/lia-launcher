use crate::ctx::Ctx;
use crate::error::{Context, Result};
use crate::logparse::{self, CrashInfo, LogLine};
use crate::mojang::install::{LegacyAssets, Prepared};
use crate::mojang::rules::{self, Env};
use crate::mojang::version::{Arg, ArgValue};
use crate::paths::Paths;
use crate::util;
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

pub const LAUNCHER_NAME: &str = "lia-launcher";

/// Прапорці JVM офіційного лаунчера — перевірені роками, підходять і для Java 8, і для 25.
const DEFAULT_GC: &[&str] = &[
    "-XX:+UnlockExperimentalVMOptions",
    "-XX:+UseG1GC",
    "-XX:G1NewSizePercent=20",
    "-XX:G1ReservePercent=20",
    "-XX:MaxGCPauseMillis=50",
    "-XX:G1HeapRegionSize=32M",
];

#[derive(Clone, Debug)]
pub struct Auth {
    pub name: String,
    /// Без дефісів.
    pub uuid: String,
    pub token: String,
    /// "msa" для Microsoft, "legacy" для офлайн.
    pub user_type: &'static str,
    pub xuid: String,
}

pub struct Options {
    pub game_dir: PathBuf,
    pub natives_dir: PathBuf,
    pub memory_mb: u32,
    pub jvm_args: Vec<String>,
    pub resolution: (u32, u32),
    pub fullscreen: bool,
    pub server: Option<(String, u16)>,
    /// Тека світу для швидкого входу в одиночну гру (1.20+).
    pub world: Option<String>,
    pub auth: Auth,
}

/// Розбиває рядок аргументів JVM з налаштувань з урахуванням лапок.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote = None;
    let mut has = false;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                has = true;
            }
            (None, c) if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn substitute(s: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        match after.find('}') {
            Some(j) => {
                let key = &after[..j];
                match vars.get(key) {
                    Some(v) => out.push_str(v),
                    None => out.push_str(&rest[i..i + 3 + j]),
                }
                rest = &after[j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn push_args(list: &[Arg], env: &Env, vars: &HashMap<&str, String>, out: &mut Vec<String>) {
    for a in list {
        match a {
            Arg::Plain(s) => out.push(substitute(s, vars)),
            Arg::Ruled { rules: r, value } => {
                if rules::allowed(r, env) {
                    match value {
                        ArgValue::One(s) => out.push(substitute(s, vars)),
                        ArgValue::Many(v) => out.extend(v.iter().map(|s| substitute(s, vars))),
                    }
                }
            }
        }
    }
}

/// Аргументи для `java` (усе після шляху до виконуваного файлу).
pub fn build_args(prep: &Prepared, paths: &Paths, o: &Options) -> Vec<String> {
    let json = &prep.json;
    let quick_play = o.server.is_some() && json.supports_quick_play();
    let quick_world = o.world.is_some() && json.supports_quick_play();
    let env = Env::current()
        .with_feature("has_custom_resolution", !o.fullscreen)
        .with_feature("is_quick_play_multiplayer", quick_play)
        .with_feature("is_quick_play_singleplayer", quick_world);
    let sep = if cfg!(windows) { ";" } else { ":" };
    let classpath = prep.classpath.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(sep);
    let game_assets = match &prep.legacy_assets {
        Some(LegacyAssets::Virtual(dir)) => dir.clone(),
        Some(LegacyAssets::Resources(_)) => o.game_dir.join("resources"),
        None => paths.assets(),
    };
    // Швидкий вхід шукає сервер у списку за точним рядком адреси, а там стандартний порт не пишуть.
    // Не знайшовши, гра заводить прихований запис без дозволу на ресурспак сервера, і пак не ставиться.
    let server_addr = o.server.as_ref().map(|(h, p)| if *p == 25565 { h.clone() } else { format!("{h}:{p}") }).unwrap_or_default();

    let vars: HashMap<&str, String> = HashMap::from([
        ("auth_player_name", o.auth.name.clone()),
        ("version_name", json.id.clone()),
        ("game_directory", o.game_dir.display().to_string()),
        ("assets_root", paths.assets().display().to_string()),
        ("game_assets", game_assets.display().to_string()),
        ("assets_index_name", prep.asset_index.clone()),
        ("auth_uuid", o.auth.uuid.clone()),
        ("auth_access_token", o.auth.token.clone()),
        ("auth_session", format!("token:{}:{}", o.auth.token, o.auth.uuid)),
        ("auth_xuid", o.auth.xuid.clone()),
        ("clientid", LAUNCHER_NAME.to_string()),
        ("user_type", o.auth.user_type.to_string()),
        ("user_properties", "{}".to_string()),
        ("version_type", json.kind.clone().unwrap_or_else(|| "release".into())),
        ("resolution_width", o.resolution.0.to_string()),
        ("resolution_height", o.resolution.1.to_string()),
        ("natives_directory", o.natives_dir.display().to_string()),
        ("launcher_name", LAUNCHER_NAME.to_string()),
        ("launcher_version", env!("CARGO_PKG_VERSION").to_string()),
        ("classpath", classpath),
        ("classpath_separator", sep.to_string()),
        ("library_directory", paths.libraries().display().to_string()),
        ("quickPlayMultiplayer", server_addr),
        ("quickPlaySingleplayer", o.world.clone().unwrap_or_default()),
    ]);

    let mut args = vec![format!("-Xmx{}M", o.memory_mb)];
    let user_gc = o.jvm_args.iter().any(|a| a.starts_with("-XX:+Use") && a.ends_with("GC"));
    if !user_gc {
        args.extend(DEFAULT_GC.iter().map(|s| s.to_string()));
    }
    // Щоб кирилиця в логах не перетворювалась на «кракозябри» (Java 18+; старші ігнорують).
    args.push("-Dstdout.encoding=UTF-8".into());
    args.push("-Dstderr.encoding=UTF-8".into());
    args.extend(o.jvm_args.iter().cloned());

    match json.arguments.as_ref().filter(|a| !a.jvm.is_empty()) {
        Some(a) => push_args(&a.jvm, &env, &vars, &mut args),
        None => {
            if cfg!(windows) {
                args.push("-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump".into());
            }
            for s in ["-Djava.library.path=${natives_directory}", "-Dminecraft.launcher.brand=${launcher_name}", "-Dminecraft.launcher.version=${launcher_version}", "-cp", "${classpath}"] {
                args.push(substitute(s, &vars));
            }
        }
    }
    if let Some((path, arg)) = &prep.log_config {
        args.push(substitute(arg, &HashMap::from([("path", path.display().to_string())])));
    }
    args.push(json.main_class.clone().unwrap_or_else(|| "net.minecraft.client.main.Main".into()));

    let legacy = match (&json.arguments, &json.minecraft_arguments) {
        (Some(a), _) if !a.game.is_empty() => {
            push_args(&a.game, &env, &vars, &mut args);
            false
        }
        (_, Some(legacy)) => {
            args.extend(legacy.split_whitespace().map(|s| substitute(s, &vars)));
            true
        }
        _ => false,
    };
    let modern_main = json.main_class.as_deref() != Some("net.minecraft.launchwrapper.Launch") || json.minecraft_arguments.as_deref().is_some_and(|m| m.contains("--username"));
    if o.fullscreen && modern_main {
        args.push("--fullscreen".into());
    }
    // У minecraftArguments (до 1.13) немає правила has_custom_resolution, а Main з 1.6 розуміє --width/--height.
    if legacy && modern_main && !o.fullscreen {
        args.extend(["--width".into(), o.resolution.0.to_string(), "--height".into(), o.resolution.1.to_string()]);
    }
    if let (Some((host, port)), false) = (&o.server, quick_play) {
        if modern_main {
            args.extend(["--server".into(), host.clone(), "--port".into(), port.to_string()]);
        }
    }
    args
}

// ---------------------------------------------------------------- запущені ігри

pub struct Game {
    pub instance_id: String,
    pub pid: u32,
    pub started: i64,
    pub log: Mutex<VecDeque<LogLine>>,
    kill: tokio::sync::Notify,
    killed: AtomicBool,
}

static RUNNING: LazyLock<Mutex<HashMap<String, Arc<Game>>>> = LazyLock::new(Default::default);
/// Остання завершена сесія кожної збірки — щоб лог було видно й після падіння.
static FINISHED: LazyLock<Mutex<HashMap<String, Arc<Game>>>> = LazyLock::new(Default::default);
const LOG_CAP: usize = 30_000;

pub fn running() -> Vec<Arc<Game>> {
    RUNNING.lock().unwrap().values().cloned().collect()
}

pub fn get(instance_id: &str) -> Option<Arc<Game>> {
    RUNNING.lock().unwrap().get(instance_id).cloned()
}

/// Лог поточної або останньої сесії збірки.
pub fn log_of(instance_id: &str) -> Vec<LogLine> {
    let g = get(instance_id).or_else(|| FINISHED.lock().unwrap().get(instance_id).cloned());
    g.map(|g| g.log.lock().unwrap().iter().cloned().collect()).unwrap_or_default()
}

pub fn kill(instance_id: &str) -> bool {
    match get(instance_id) {
        Some(g) => {
            g.killed.store(true, Ordering::Relaxed);
            g.kill.notify_one();
            true
        }
        None => false,
    }
}

/// Файл у теці збірки, поки гра працює: лаунчер, перезапущений посеред гри, бачить її
/// і не дасть запустити ту саму збірку вдруге (два процеси в одному світі ламають його).
pub const RUNNING_FILE: &str = ".running";

fn java_alive(pid: u32, started: i64) -> bool {
    let mut sys = sysinfo::System::new();
    let p = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[p]), true);
    sys.process(p).is_some_and(|proc_| proc_.name().to_string_lossy().to_lowercase().contains("java") && same_start(proc_.start_time(), started))
}

/// Після перезавантаження ПК pid з `.running` може дістатися чужій Java, тому звіряємо час старту (0 означає невідомо).
fn same_start(start_secs: u64, started_ms: i64) -> bool {
    start_secs == 0 || (start_secs as i64 * 1000 - started_ms).abs() < 60_000
}

/// Підхоплює гру, запущену попереднім сеансом лаунчера (без логу — лише стан і «Зупинити»).
pub fn adopt(ctx: &Ctx, instance_id: &str, instance_dir: &Path) {
    let marker = instance_dir.join(RUNNING_FILE);
    let Ok(v) = std::fs::read_to_string(&marker).map(|s| serde_json::from_str::<serde_json::Value>(&s).unwrap_or_default()) else { return };
    let pid = v["pid"].as_u64().unwrap_or(0) as u32;
    let started = v["started"].as_i64().unwrap_or(0);
    if pid == 0 || get(instance_id).is_some() || !java_alive(pid, started) {
        let _ = std::fs::remove_file(&marker);
        return;
    }
    let game = Arc::new(Game {
        instance_id: instance_id.to_string(),
        pid,
        started,
        log: Mutex::new(VecDeque::new()),
        kill: tokio::sync::Notify::new(),
        killed: AtomicBool::new(false),
    });
    RUNNING.lock().unwrap().insert(instance_id.to_string(), game.clone());
    let ctx = ctx.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(3)) => {}
                _ = game.kill.notified() => {
                    let mut sys = sysinfo::System::new();
                    let p = sysinfo::Pid::from_u32(game.pid);
                    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[p]), true);
                    if let Some(proc_) = sys.process(p) {
                        proc_.kill();
                    }
                }
            }
            if !java_alive(game.pid, game.started) {
                break;
            }
        }
        RUNNING.lock().unwrap().remove(&game.instance_id);
        let _ = std::fs::remove_file(&marker);
        ctx.emit("game", json!({ "instanceId": game.instance_id, "state": "exited", "code": null, "killed": game.killed.load(Ordering::Relaxed), "crash": null }));
    });
}

pub struct Exit {
    pub instance_id: String,
    pub code: Option<i32>,
    pub started: i64,
    pub crash: Option<CrashInfo>,
}

/// Запускає гру й стежить за нею: лог у подіях `game-log`, стан у `game`.
pub fn spawn(
    ctx: &Ctx,
    instance_id: &str,
    java: &Path,
    args: Vec<String>,
    game_dir: &Path,
    natives_dir: PathBuf,
    on_exit: impl FnOnce(Exit) + Send + 'static,
) -> Result<Arc<Game>> {
    std::fs::create_dir_all(game_dir).ctx(|| game_dir.display().to_string())?;
    let mut cmd = tokio::process::Command::new(java);
    cmd.args(&args).current_dir(game_dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = cmd.spawn().ctx(|| java.display().to_string())?;
    let started = util::now_ms();
    let game = Arc::new(Game {
        instance_id: instance_id.to_string(),
        pid: child.id().unwrap_or(0),
        started,
        log: Mutex::new(VecDeque::new()),
        kill: tokio::sync::Notify::new(),
        killed: AtomicBool::new(false),
    });
    RUNNING.lock().unwrap().insert(instance_id.to_string(), game.clone());
    ctx.emit("game", json!({ "instanceId": instance_id, "state": "running", "pid": game.pid, "started": started }));
    let marker = game_dir.parent().map(|d| d.join(RUNNING_FILE));
    if let Some(m) = &marker {
        let _ = std::fs::write(m, json!({ "pid": game.pid, "started": started }).to_string());
    }

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<(bool, Vec<u8>)>();
    if let Some(out) = child.stdout.take() {
        tokio::spawn(read_lines(out, false, tx.clone()));
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(read_lines(err, true, tx.clone()));
    }
    drop(tx);
    let pump = tokio::spawn(pump_logs(ctx.clone(), game.clone(), rx));

    let ctx = ctx.clone();
    let game2 = game.clone();
    let game_dir = game_dir.to_path_buf();
    tokio::spawn(async move {
        let status = tokio::select! {
            s = child.wait() => s,
            _ = game2.kill.notified() => {
                let _ = child.start_kill();
                child.wait().await
            }
        };
        // Дочірній процес гри (успадкував stdout) може тримати канал відкритим і після її виходу.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), pump).await;
        let code = status.ok().and_then(|s| s.code());
        let killed = game2.killed.load(Ordering::Relaxed);
        let crash = if code.unwrap_or(0) != 0 && !killed { Some(crash_info(&game2, &game_dir)) } else { None };
        RUNNING.lock().unwrap().remove(&game2.instance_id);
        FINISHED.lock().unwrap().insert(game2.instance_id.clone(), game2.clone());
        if let Some(m) = &marker {
            let _ = std::fs::remove_file(m);
        }
        ctx.emit("game", json!({ "instanceId": game2.instance_id, "state": "exited", "code": code, "killed": killed, "crash": crash }));
        // Нативні .dll звільняються не миттєво після виходу процесу.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let _ = std::fs::remove_dir_all(&natives_dir);
        on_exit(Exit { instance_id: game2.instance_id.clone(), code, started, crash });
    });
    Ok(game)
}

async fn read_lines(stream: impl AsyncRead + Unpin, stderr: bool, tx: tokio::sync::mpsc::UnboundedSender<(bool, Vec<u8>)>) {
    let mut reader = BufReader::new(stream);
    loop {
        let mut buf = Vec::new();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                while matches!(buf.last(), Some(b'\n' | b'\r')) {
                    buf.pop();
                }
                if tx.send((stderr, buf)).is_err() {
                    break;
                }
            }
        }
    }
}

/// Збирає рядки пачками раз на 100 мс — інтерфейс не захлинається від тисяч подій.
async fn pump_logs(ctx: Ctx, game: Arc<Game>, mut rx: tokio::sync::mpsc::UnboundedReceiver<(bool, Vec<u8>)>) {
    // Свій розбирач на кожен потік: рядки stderr приходять упереміш із багаторядковими подіями stdout.
    let mut parsers: [logparse::Parser; 2] = Default::default();
    let mut batch: Vec<LogLine> = Vec::new();
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Some((stderr, bytes)) => {
                    let line = decode(&bytes);
                    if let Some(mut l) = parsers[stderr as usize].feed(&line, stderr) {
                        if l.t == 0 {
                            l.t = util::now_ms();
                        }
                        batch.push(l);
                    }
                }
                None => break,
            },
            _ = tick.tick() => flush(&ctx, &game, &mut batch),
        }
    }
    flush(&ctx, &game, &mut batch);
}

/// Java 8 пише stderr у кодуванні системи (cp1251 на українській Windows), новіші — у UTF-8.
pub(crate) fn decode(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Globalization::{MultiByteToWideChar, CP_ACP};
        let n = MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), bytes.len() as i32, std::ptr::null_mut(), 0);
        if n > 0 {
            let mut wide = vec![0u16; n as usize];
            MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), bytes.len() as i32, wide.as_mut_ptr(), n);
            return String::from_utf16_lossy(&wide);
        }
    }
    String::from_utf8_lossy(bytes).to_string()
}

fn flush(ctx: &Ctx, game: &Game, batch: &mut Vec<LogLine>) {
    if batch.is_empty() {
        return;
    }
    {
        let mut log = game.log.lock().unwrap();
        log.extend(batch.iter().cloned());
        let excess = log.len().saturating_sub(LOG_CAP);
        log.drain(..excess);
    }
    ctx.emit("game-log", json!({ "instanceId": game.instance_id, "lines": batch }));
    batch.clear();
}

fn crash_info(game: &Game, game_dir: &Path) -> CrashInfo {
    let log: String = {
        let l = game.log.lock().unwrap();
        l.iter().rev().take(600).collect::<Vec<_>>().into_iter().rev().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n")
    };
    let newest = |dir: PathBuf, prefix: &str| -> Option<PathBuf> {
        std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
            .filter_map(|e| {
                let m = e.metadata().ok()?.modified().ok()?;
                let ms = m.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
                (ms >= game.started - 5_000).then_some((ms, e.path()))
            })
            .max_by_key(|(ms, _)| *ms)
            .map(|(_, p)| p)
    };
    let report_path = newest(game_dir.join("crash-reports"), "crash-").or_else(|| newest(game_dir.to_path_buf(), "hs_err_pid"));
    // hs_err_pid пишеться в кодуванні системи (кирилиця в шляху), і read_to_string на ньому падає.
    let report = report_path.as_ref().and_then(|p| std::fs::read(p).ok()).map(|b| String::from_utf8_lossy(&b).into_owned());
    let mut info = logparse::analyze(&log, report.as_deref());
    info.report_path = report_path.map(|p| p.display().to_string());
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split() {
        assert_eq!(split_args(r#"-Xss2M  -Dfoo="a b" -Dx='' "#), vec!["-Xss2M", "-Dfoo=a b", "-Dx="]);
        assert!(split_args("   ").is_empty());
    }

    #[test]
    fn subst() {
        let vars = HashMap::from([("a", "1".to_string()), ("library_directory", "C:/L".to_string())]);
        assert_eq!(substitute("x${a}y${a}", &vars), "x1y1");
        assert_eq!(substitute("${library_directory}/f.jar", &vars), "C:/L/f.jar");
        assert_eq!(substitute("${unknown}-${a}", &vars), "${unknown}-1");
        assert_eq!(substitute("tail ${", &vars), "tail ${");
    }

    /// Рядок stderr посеред XML-події stdout не має ні губитися, ні ламати подію.
    #[tokio::test]
    async fn streams_dont_mix() {
        let ctx = Ctx { paths: Paths::new(std::env::temp_dir()), http: crate::net::client(), emit: Arc::new(|_, _| {}), concurrency: 1 };
        let game = Arc::new(Game { instance_id: "t".into(), pid: 0, started: 0, log: Default::default(), kill: tokio::sync::Notify::new(), killed: AtomicBool::new(false) });
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        for (stderr, l) in [
            (false, r#"<log4j:Event logger="x" timestamp="1" level="INFO" thread="main">"#),
            (true, "java.lang.RuntimeException: boom"),
            (false, "  <log4j:Message><![CDATA[hi]]></log4j:Message>"),
            (false, "</log4j:Event>"),
        ] {
            tx.send((stderr, l.as_bytes().to_vec())).unwrap();
        }
        drop(tx);
        pump_logs(ctx, game.clone(), rx).await;
        let texts: Vec<_> = game.log.lock().unwrap().iter().map(|l| l.text.clone()).collect();
        assert_eq!(texts, vec!["java.lang.RuntimeException: boom", "hi"]);
    }

    #[test]
    fn pid_reuse() {
        assert!(same_start(1_700_000_001, 1_700_000_000_500));
        assert!(same_start(0, 1_700_000_000_500));
        assert!(!same_start(1_700_090_000, 1_700_000_000_500));
    }

    fn opts(fullscreen: bool) -> Options {
        Options {
            game_dir: "g".into(),
            natives_dir: "n".into(),
            memory_mb: 1024,
            jvm_args: vec![],
            resolution: (1280, 720),
            fullscreen,
            server: None,
            world: None,
            auth: Auth { name: "A".into(), uuid: "u".into(), token: "0".into(), user_type: "legacy", xuid: "0".into() },
        }
    }

    fn prep(json: &str) -> Prepared {
        Prepared { json: serde_json::from_str(json).unwrap(), classpath: vec![], natives: vec![], asset_index: "x".into(), legacy_assets: None, log_config: None }
    }

    #[test]
    fn window_size() {
        let paths = Paths::new("r".into());
        let count = |a: &[String]| a.iter().filter(|s| *s == "--width").count();
        // Старі версії (minecraftArguments) не мають правила has_custom_resolution, тож розмір додаємо самі.
        let legacy = prep(r#"{"id":"1.12.2","mainClass":"net.minecraft.client.main.Main","minecraftArguments":"--username ${auth_player_name}"}"#);
        let a = build_args(&legacy, &paths, &opts(false));
        assert!(a.ends_with(&["--width".into(), "1280".into(), "--height".into(), "720".into()]), "{a:?}");
        assert_eq!(count(&build_args(&legacy, &paths, &opts(true))), 0);
        let modern = prep(
            r#"{"id":"1.21","mainClass":"net.minecraft.client.main.Main","arguments":{"game":["--username","${auth_player_name}",
                {"rules":[{"action":"allow","features":{"has_custom_resolution":true}}],"value":["--width","${resolution_width}","--height","${resolution_height}"]}]}}"#,
        );
        assert_eq!(count(&build_args(&modern, &paths, &opts(false))), 1);
        // До 1.6 (аплет через launchwrapper) не чіпаємо.
        let applet = prep(r#"{"id":"1.5.2","mainClass":"net.minecraft.launchwrapper.Launch","minecraftArguments":"${auth_player_name} ${auth_session}"}"#);
        assert_eq!(count(&build_args(&applet, &paths, &opts(false))), 0);
    }
}
