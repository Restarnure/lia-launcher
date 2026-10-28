//! Соцмережа ЛІЇ наживо, з бойовим сервером (LIA_API, типово https://lia.starnure.dev). Лише вручну:
//!   cargo test --test lia -- --ignored --nocapture --test-threads=1
//!
//! `world_through_relay` — те, що відбувається, коли гравець відкриває світ друзям: сервер Minecraft
//! з authlib-injector в online-mode (як LAN-світ хоста), відкритий через ретранслятор, і другий гравець
//! ЛІЯ ID, що заходить за адресою світу. Потрібні:
//!   LIA_TEST_SERVER_JAR — paper-1.21.4-*.jar, LIA_TEST_JAVA — java 21, LIA_TEST_BOT — bot.js
//!   (minecraft-protocol; входить логіном/паролем через Yggdrasil ЛІЇ, друкує BOT_OK).
//! Використовує тестові акаунти ЛІЯ ID LiaTestHost і LiaTestGuest (створює, якщо їх немає) і нік LiaTestPirate.
//!
//! `pack_roundtrip` — експорт збірки в .mrpack (мод з Modrinth посиланням, свій мод і config файлами)
//! і встановлення назад, як це робить друг.

use lia_lib::ctx::Ctx;
use lia_lib::instances::{self, NewInstance};
use lia_lib::paths::Paths;
use lia_lib::{lia, modrinth};
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn ctx(name: &str) -> Ctx {
    let dir = std::env::temp_dir().join("lia-social-test").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    Ctx { paths: Paths::new(dir), http: lia_lib::net::client(), emit: Arc::new(|_, _| {}), concurrency: 16 }
}

fn env(k: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| panic!("потрібна змінна {k}"))
}

// Багатопотоковий runtime: бот запускається блокуючим викликом, а ретранслятор хоста має працювати паралельно.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn world_through_relay() {
    let ctx = ctx("relay");
    // Ті самі тестові акаунти між прогонами: на сервері ліміт реєстрацій з однієї IP.
    let account = |name: &'static str, pass: &'static str| {
        let http = ctx.http.clone();
        async move {
            match lia::login(&http, name, pass, None).await {
                Ok(s) => s,
                Err(_) => lia::login(&http, name, pass, Some(&format!("{}@test.lia", name.to_lowercase()))).await.expect("реєстрація"),
            }
        }
    };
    let host = account("LiaTestHost", "test-password-1").await;
    let guest = account("LiaTestGuest", "test-password-2").await;
    // Гість — офлайн-нік; гра входила б через ЛІЮ так само.
    let (guest_token, guest_uuid, guest_name) = lia::game_session(&ctx.http, &guest, &lia_lib::accounts::offline("LiaTestPirate").unwrap()).await.expect("сесія гостя");

    // Ті самі аргументи JVM, що й у гри з ЛІЯ ID.
    let jar = lia::injector(&ctx).await.expect("authlib-injector");
    let args = lia::jvm_args(&jar);
    let dir = ctx.paths.root.join("server");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("eula.txt"), "eula=true\n").unwrap();
    std::fs::write(dir.join("server.properties"), "online-mode=true\nserver-port=25599\nlevel-type=minecraft\\:flat\nspawn-protection=0\n").unwrap();
    let mut server = Command::new(env("LIA_TEST_JAVA"))
        .args(&args)
        .args(["-Xmx1G", "-jar", &env("LIA_TEST_SERVER_JAR"), "nogui"])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("сервер");
    let out = server.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for l in BufReader::new(out).lines().map_while(Result::ok) {
            let _ = tx.send(l);
        }
    });
    let t0 = Instant::now();
    let mut injector_ok = false;
    loop {
        let l = rx.recv_timeout(Duration::from_secs(240)).expect("сервер не стартував");
        if l.contains("authlib-injector") {
            println!("  {l}");
        }
        injector_ok |= l.contains("authlib-injector") && (l.contains("Logged in") || l.contains("lia.starnure.dev") || l.contains("ЛІЯ"));
        if l.contains("Done (") {
            break;
        }
    }
    println!("  сервер готовий за {:?}", t0.elapsed());

    let token = host.token.clone();
    let (code, address) = lia::share("test", &token, 25599, || {}).await.expect("ретранслятор");
    println!("  світ відкрито: {code} → {address}");
    let (h, p) = address.split_once(':').unwrap();
    let bot = Command::new("node").args([&env("LIA_TEST_BOT"), h, p, &guest_name, &guest_token, &guest_uuid, &lia::api()]).output().expect("бот");
    let bot_out = String::from_utf8_lossy(&bot.stdout).to_string();
    println!("  бот: {}", bot_out.trim());

    // Сервер мусить побачити гравця з профілем ЛІЯ ID (UUID з сервера ЛІЇ).
    let mut joined = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    while let Ok(l) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        if l.contains(&guest_name) {
            println!("  {l}");
            joined |= l.contains("logged in") || l.contains("joined the game") || l.contains("UUID of player");
        }
    }
    lia::unshare("test");
    if let Some(mut stdin) = server.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(b"stop\n");
    }
    let _ = server.wait();
    let _ = injector_ok;
    assert!(bot_out.contains("BOT_OK"), "бот не зайшов: {bot_out}");
    assert!(joined, "сервер не прийняв гравця ЛІЯ ID");
}

#[tokio::test]
#[ignore]
async fn pack_roundtrip() {
    let ctx = ctx("pack");
    let mut inst = instances::create(&ctx.paths, NewInstance { name: "Тест".into(), mc_version: "1.21.4".into(), loader: "fabric".into(), loader_version: Some("0.16.10".into()), icon: None }).unwrap();
    inst.loader_version = Some("0.16.10".into());
    let game = ctx.paths.game_dir(&inst.id);
    // Мод з Modrinth (Fabric API для 1.21.4) — має піти посиланням.
    let ver: serde_json::Value = ctx
        .http
        .get("https://api.modrinth.com/v2/project/fabric-api/version?game_versions=%5B%221.21.4%22%5D&loaders=%5B%22fabric%22%5D")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let f = &ver[0]["files"][0];
    let bytes = ctx.http.get(f["url"].as_str().unwrap()).send().await.unwrap().bytes().await.unwrap();
    std::fs::create_dir_all(game.join("mods")).unwrap();
    std::fs::write(game.join("mods").join(f["filename"].as_str().unwrap()), &bytes).unwrap();
    std::fs::write(game.join("mods").join("my-own-mod.jar"), b"not on modrinth").unwrap();
    std::fs::write(game.join("mods").join("off.jar.disabled"), b"disabled").unwrap();
    std::fs::create_dir_all(game.join("config").join("sub")).unwrap();
    std::fs::write(game.join("config").join("sub").join("a.toml"), b"x = 1").unwrap();

    let pack = lia::export_mrpack(&ctx, &inst).await.expect("експорт");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(pack.clone())).unwrap();
    let mut index = String::new();
    zip.by_name("modrinth.index.json").unwrap().read_to_string(&mut index).unwrap();
    let index: serde_json::Value = serde_json::from_str(&index).unwrap();
    println!("  index: {index}");
    assert_eq!(index["dependencies"]["fabric-loader"], "0.16.10");
    assert_eq!(index["files"].as_array().unwrap().len(), 1, "Fabric API має бути посиланням");
    assert!(index["files"][0]["downloads"][0].as_str().unwrap().starts_with("https://cdn.modrinth.com/"));
    assert!(zip.by_name("overrides/mods/my-own-mod.jar").is_ok());
    assert!(zip.by_name("overrides/config/sub/a.toml").is_ok());
    assert!(zip.by_name("overrides/mods/off.jar.disabled").is_err());

    let path: PathBuf = ctx.paths.root.join("p.mrpack");
    std::fs::write(&path, &pack).unwrap();
    let task = ctx.task("import");
    let back = modrinth::import_mrpack(&ctx, &path, None, &task).await.expect("імпорт");
    let g2 = ctx.paths.game_dir(&back.id);
    assert_eq!(std::fs::read(g2.join("mods").join(f["filename"].as_str().unwrap())).unwrap(), bytes.to_vec());
    assert_eq!(std::fs::read(g2.join("mods").join("my-own-mod.jar")).unwrap(), b"not on modrinth");
    assert_eq!(std::fs::read(g2.join("config").join("sub").join("a.toml")).unwrap(), b"x = 1");
    assert_eq!(back.loader, "fabric");
    assert!(back.version_id.is_some(), "модлоадер не встановлено");
    println!("  збірку відновлено: {} ({})", back.name, back.version_id.unwrap());
}
