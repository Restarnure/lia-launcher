//! Справжній запуск гри: завантаження версії, Java, старт до головного меню.
//! Мережа і ~1 ГБ на диску, тому лише вручну:
//!   cargo test --test e2e -- --ignored --nocapture
//! Версії: змінна LIA_E2E (через кому), напр. "26.3,1.12.2,fabric:1.21.11".
//! Дані — у %TEMP%\lia-e2e (або LIA_E2E_DIR). LIA_E2E_LIA=1 — запуск з ЛІЯ ID (тестовий акаунт з tests/lia.rs).

use lia_lib::ctx::Ctx;
use lia_lib::launch::{self, Auth, Options};
use lia_lib::mojang::{install, java};
use lia_lib::paths::Paths;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Рядки, які гра пише вже після ініціалізації вікна й звуку — тобто вона дійшла до меню.
const READY: &[&str] = &["Sound engine started", "SoundSystem initialized", "Starting up SoundSystem", "OpenAL initialized", "Created: 1024x"];

fn ctx(logs: Arc<Mutex<Vec<String>>>) -> Ctx {
    let dir = std::env::var("LIA_E2E_DIR").map(Into::into).unwrap_or_else(|_| std::env::temp_dir().join("lia-e2e"));
    Ctx {
        paths: Paths::new(dir),
        http: lia_lib::net::client(),
        emit: Arc::new(move |ev, payload| match ev {
            "game-log" => {
                for l in payload["lines"].as_array().unwrap() {
                    logs.lock().unwrap().push(format!("[{}] {}", l["level"].as_str().unwrap_or(""), l["text"].as_str().unwrap_or("")));
                }
            }
            "task" if payload["state"] != "running" || payload["done"] == 0 => {
                println!("  task {} {} {}/{}", payload["stage"], payload["state"], payload["done"], payload["total"]);
            }
            _ => {}
        }),
        concurrency: 24,
    }
}

async fn run_version(spec: &str) -> Result<(), String> {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let ctx = ctx(logs.clone());
    let r = run_with(spec, &ctx, logs.clone()).await;
    let _ = std::fs::write(ctx.paths.root.join(format!("{}.log", spec.replace(':', "-"))), logs.lock().unwrap().join("\n"));
    r
}

async fn run_with(spec: &str, ctx: &Ctx, logs: Arc<Mutex<Vec<String>>>) -> Result<(), String> {
    let ctx = ctx.clone();
    let (loader, mc) = spec.split_once(':').unwrap_or(("vanilla", spec));
    let version_id = if loader == "vanilla" {
        mc.to_string()
    } else {
        let task = ctx.task("loader");
        lia_lib::loaders::install(&ctx, loader, mc, None, &task).await.map_err(|e| format!("loader: {e}"))?
    };
    let t0 = Instant::now();
    let task = ctx.task(spec);
    let prep = install::prepare(&ctx, &version_id, &task, false).await.map_err(|e| format!("prepare: {e}"))?;
    let java = java::ensure_runtime(&ctx, &prep.json.java_component(), &task, false).await.map_err(|e| format!("java: {e}"))?;
    println!("  {spec}: підготовка {:?}, java {}", t0.elapsed(), java.display());

    let game_dir = ctx.paths.root.join("instances").join(spec.replace(':', "-")).join(".minecraft");
    let natives_dir = ctx.paths.natives_root().join(format!("e2e-{}", spec.replace(':', "-")));
    install::extract_natives(&prep.natives, &natives_dir).map_err(|e| e.to_string())?;
    if let Some(install::LegacyAssets::Resources(pairs)) = &prep.legacy_assets {
        install::copy_assets(pairs, &game_dir.join("resources"), &task).map_err(|e| e.to_string())?;
    }
    // LIA_E2E_LIA=1 — гра з ЛІЯ ID: authlib-injector, свій http.agent і токен тестового акаунта.
    let (jvm_args, auth) = if std::env::var("LIA_E2E_LIA").is_ok() {
        let s = lia_lib::lia::login(&ctx.http, "LiaTestHost", "test-password-1", false).await.map_err(|e| format!("ЛІЯ ID: {e}"))?;
        let nick = lia_lib::accounts::offline("LiaTestNick").map_err(|e| e.to_string())?;
        let (token, uuid, name) = lia_lib::lia::game_session(&ctx.http, &s, &nick).await.map_err(|e| format!("сесія гри: {e}"))?;
        let jar = lia_lib::lia::injector(&ctx).await.map_err(|e| format!("injector: {e}"))?;
        (lia_lib::lia::jvm_args(&jar).to_vec(), Auth { name, uuid, token, user_type: "mojang", xuid: "0".into() })
    } else {
        (vec![], Auth { name: "LiaTest".into(), uuid: lia_lib::accounts::offline_uuid("LiaTest"), token: "0".into(), user_type: "legacy", xuid: "0".into() })
    };
    let opts = Options {
        game_dir: game_dir.clone(),
        natives_dir: natives_dir.clone(),
        memory_mb: 2048,
        jvm_args,
        resolution: (854, 480),
        fullscreen: false,
        server: None,
        world: None,
        auth,
    };
    let args = launch::build_args(&prep, &ctx.paths, &opts);
    let (tx, rx) = std::sync::mpsc::channel();
    let game = launch::spawn(&ctx, spec, &java, args, &game_dir, natives_dir, move |exit| {
        let _ = tx.send(exit.code);
    })
    .map_err(|e| e.to_string())?;

    let deadline = Instant::now() + Duration::from_secs(150);
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Ok(code) = rx.try_recv() {
            let tail = logs.lock().unwrap().iter().rev().take(40).rev().cloned().collect::<Vec<_>>().join("\n");
            return Err(format!("гра завершилась з кодом {code:?}\n{tail}"));
        }
        if logs.lock().unwrap().iter().any(|l| READY.iter().any(|m| l.contains(m))) {
            // Маркер — ще не меню: гра має прожити далі без падіння.
            tokio::time::sleep(Duration::from_secs(8)).await;
            if let Ok(code) = rx.try_recv() {
                let tail = logs.lock().unwrap().iter().rev().take(40).rev().cloned().collect::<Vec<_>>().join("\n");
                return Err(format!("гра впала після старту, код {code:?}\n{tail}"));
            }
            let tail = logs.lock().unwrap().iter().rev().take(6).rev().cloned().collect::<Vec<_>>().join("\n    ");
            println!("  {spec}: OK за {:?} (pid {}), останні рядки:\n    {tail}", t0.elapsed(), game.pid);
            launch::kill(spec);
            let _ = tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(20))).await;
            return Ok(());
        }
        if Instant::now() > deadline {
            launch::kill(spec);
            let tail = logs.lock().unwrap().iter().rev().take(40).rev().cloned().collect::<Vec<_>>().join("\n");
            return Err(format!("за 150 с гра не дійшла до меню\n{tail}"));
        }
    }
}

#[tokio::test]
#[ignore]
async fn launch_versions() {
    let list = std::env::var("LIA_E2E").unwrap_or_else(|_| "26.3".into());
    let mut failed = Vec::new();
    for spec in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        println!("== {spec}");
        let r = run_version(spec).await;
        if let Err(e) = r {
            println!("  ПОМИЛКА {spec}: {e}");
            failed.push(spec.to_string());
        }
    }
    assert!(failed.is_empty(), "не запустились: {failed:?}");
}
