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
//! `pack_roundtrip`: автор публікує збірку (мод з Modrinth посиланням, свої файли на сервер ЛІЇ),
//! друг встановлює її, автор додає русифікатор, друг оновлюється й отримує лише змінене.
//! Підійде локальний сервер: LIA_API=http://127.0.0.1:<порт>.

use lia_lib::ctx::Ctx;
use lia_lib::instances::{self, NewInstance};
use lia_lib::paths::Paths;
use lia_lib::lia;
use lia_lib::packs::{self, Kind};
use std::io::{BufRead, BufReader};
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


/// Ті самі тестові акаунти між прогонами: на сервері ліміт реєстрацій з однієї IP.
async fn account(http: &reqwest::Client, name: &str, pass: &str) -> lia::Session {
    match lia::login(http, name, pass, None).await {
        Ok(s) => s,
        Err(_) => lia::login(http, name, pass, Some(&format!("{}@test.lia", name.to_lowercase()))).await.expect("реєстрація"),
    }
}

fn put(dir: &std::path::Path, rel: &str, bytes: &[u8]) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn pack_roundtrip() {
    let author = ctx("pack-author");
    let friend = ctx("pack-friend");
    let host = account(&author.http, "LiaTestHost", "test-password-1").await;
    let guest = account(&friend.http, "LiaTestGuest", "test-password-2").await;
    let task = author.task("pack");
    let inst = instances::create(&author.paths, NewInstance { name: "Тест".into(), mc_version: "1.21.4".into(), loader: "fabric".into(), loader_version: Some("0.16.10".into()), icon: None }).unwrap();
    let game = author.paths.game_dir(&inst.id);
    // Мод з Modrinth (Fabric API для 1.21.4) має піти посиланням.
    let ver: serde_json::Value = author
        .http
        .get("https://api.modrinth.com/v2/project/fabric-api/version?game_versions=%5B%221.21.4%22%5D&loaders=%5B%22fabric%22%5D")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let f = &ver[0]["files"][0];
    let fapi = format!("mods/{}", f["filename"].as_str().unwrap());
    let fapi_bytes = author.http.get(f["url"].as_str().unwrap()).send().await.unwrap().bytes().await.unwrap();
    put(&game, &fapi, &fapi_bytes);
    put(&game, "mods/my-own-mod.jar", b"not on modrinth");
    put(&game, "mods/off.jar.disabled", b"disabled");
    put(&game, "config/sub/a.toml", b"x = 1");
    put(&game, "config/gone.toml", b"old");
    put(&game, "resourcepacks/Folder/pack.mcmeta", b"{}");
    put(&game, "saves/w/level.dat", b"world");
    put(&game, "logs/latest.log", b"log");
    put(&game, "options.txt", b"lang:uk_ua\nresourcePacks:[\"vanilla\",\"file/Folder\"]\n");

    let plan = packs::plan(&author, &inst, &host.token, &task).await.expect("план");
    let kind = |p: &str| plan.files.iter().find(|f| f.path == p).map(|f| f.kind);
    assert_eq!(kind(&fapi), Some(Kind::Link));
    assert!(matches!(kind("mods/my-own-mod.jar"), Some(Kind::Upload | Kind::Stored)));
    assert!(kind("saves/w/level.dat").is_none() && kind("logs/latest.log").is_none() && kind("options.txt").is_none());
    let pack = packs::publish(&author, &inst, &host.token, "public", "тест", None, plan.selection.clone(), true, &task).await.expect("публікація");
    let id = pack["id"].as_str().unwrap().to_string();
    println!("  опубліковано {id}");

    // Друг встановлює збірку.
    let ftask = friend.task("install");
    let mut mine = packs::install(&friend, &guest.token, &id, "LiaTestHost", &ftask).await.expect("встановлення");
    let g2 = friend.paths.game_dir(&mine.id);
    let read = |rel: &str| std::fs::read(g2.join(rel)).unwrap_or_default();
    assert_eq!(read(&fapi), fapi_bytes.to_vec());
    for (rel, want) in [("mods/my-own-mod.jar", &b"not on modrinth"[..]), ("mods/off.jar.disabled", b"disabled"), ("config/sub/a.toml", b"x = 1"), ("resourcepacks/Folder/pack.mcmeta", b"{}")] {
        assert_eq!(read(rel), want, "{rel}");
    }
    assert!(!g2.join("saves").exists() && !g2.join("logs").exists());
    let opts = String::from_utf8(read("options.txt")).unwrap();
    assert!(opts.contains("lang:uk_ua") && opts.contains(r#"resourcePacks:["vanilla","file/Folder"]"#) && opts.contains("onboardAccessibility:false"), "{opts}");
    assert!(mine.version_id.is_some(), "модлоадер не встановлено");
    assert_eq!(mine.modpack.as_ref().unwrap().version_id, "1");
    let fapi_time = std::fs::metadata(g2.join(&fapi)).unwrap().modified().unwrap();

    // Друг додав свій мод і змінив мову.
    put(&g2, "mods/friend-own.jar", b"mine");
    let opts = opts.replace("lang:uk_ua", "lang:en_us");
    std::fs::write(g2.join("options.txt"), &opts).unwrap();

    // Автор додав русифікатор (ресурспак, увімкнений у грі), змінив свій мод і прибрав конфіг.
    put(&game, "resourcepacks/ru.zip", b"russian");
    put(&game, "mods/my-own-mod.jar", b"v2");
    std::fs::remove_file(game.join("config/gone.toml")).unwrap();
    put(&game, "options.txt", b"lang:uk_ua\nresourcePacks:[\"vanilla\",\"file/Folder\",\"file/ru.zip\"]\n");
    let p2 = packs::publish(&author, &inst, &host.token, "public", "тест", Some(id.clone()), plan.selection.clone(), true, &task).await.expect("v2");
    assert_eq!(p2["version"], 2);

    packs::update(&friend, &guest.token, &mut mine, "LiaTestHost", &ftask).await.expect("оновлення");
    assert_eq!(read("resourcepacks/ru.zip"), b"russian");
    assert_eq!(read("mods/my-own-mod.jar"), b"v2");
    assert!(!g2.join("config/gone.toml").exists(), "видалений автором файл лишився");
    assert_eq!(read("mods/friend-own.jar"), b"mine", "свій мод друга зник");
    assert_eq!(std::fs::metadata(g2.join(&fapi)).unwrap().modified().unwrap(), fapi_time, "незмінений мод качався знову");
    let opts = String::from_utf8(read("options.txt")).unwrap();
    assert!(opts.contains("lang:en_us") && opts.contains(r#"resourcePacks:["vanilla","file/Folder","file/ru.zip"]"#), "{opts}");
    assert_eq!(mine.modpack.as_ref().unwrap().version_id, "2");
    println!("  оновлено до v2: русифікатор на місці й увімкнений");

    lia::call(&author.http, reqwest::Method::DELETE, &format!("/api/packs/{id}"), Some(&host.token), None).await.expect("видалення");
}

/// Справжня збірка з базовим модпаком (LIA_TEST_INSTANCE — тека збірки, напр. Prominence II). Працює з копією.
/// Автор публікує, друг встановлює з нуля, автор додає русифікатор, друг оновлюється без перекачування модпака.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn real_pack_roundtrip() {
    let author = ctx("real-author");
    let friend = ctx("real-friend");
    let src = PathBuf::from(env("LIA_TEST_INSTANCE"));
    let id = src.file_name().unwrap().to_string_lossy().to_string();
    lia_lib::util::copy_dir(&src, &author.paths.instance_dir(&id)).unwrap();
    let inst = instances::get(&author.paths, &id).unwrap();
    let game = author.paths.game_dir(&id);
    let host = account(&author.http, "LiaTestHost", "test-password-1").await;
    let guest = account(&friend.http, "LiaTestGuest", "test-password-2").await;
    let task = author.task("pack");

    let t0 = Instant::now();
    let plan = packs::plan(&author, &inst, &host.token, &task).await.expect("план");
    let sum = |k: Kind| plan.files.iter().filter(|f| f.kind == k && plan.selection.picks(&f.path)).fold((0, 0), |a, f| (a.0 + 1, a.1 + f.size));
    println!("  план за {:?}: база {:?}", t0.elapsed(), plan.base.as_ref().map(|b| (&b.name, &b.version_name)));
    for k in [Kind::Base, Kind::Link, Kind::Upload, Kind::Stored, Kind::Big] {
        println!("    {k:?}: {:?}", sum(k));
    }
    println!("    вибрано: {:?}", plan.selection.include);
    assert!(plan.base.is_some(), "збірка з Modrinth, а базу не знайдено");
    assert!(sum(Kind::Upload).1 < 100 * 1000 * 1000, "завантажилося б забагато");
    let pack = packs::publish(&author, &inst, &host.token, "public", "", None, plan.selection.clone(), true, &task).await.expect("публікація");
    let pid = pack["id"].as_str().unwrap().to_string();
    println!("  опубліковано {pid}, розмір у гравця {}", pack["size"]);

    let t0 = Instant::now();
    let ftask = friend.task("install");
    let mut mine = packs::install(&friend, &guest.token, &pid, "LiaTestHost", &ftask).await.expect("встановлення");
    println!("  друг встановив за {:?}", t0.elapsed());
    let g2 = friend.paths.game_dir(&mine.id);
    let m = packs::fetch_manifest(&friend, &guest.token, &pid).await.unwrap();
    let mut bad = vec![];
    for f in &m.files {
        let p = g2.join(&f.path);
        if f.path != "options.txt" && f.path != "servers.dat" && lia_lib::util::sha1_file(&p).ok().as_deref() != Some(f.sha1.as_str()) {
            bad.push(f.path.clone());
        }
    }
    assert!(bad.is_empty(), "не збігаються з маніфестом: {bad:?}");
    // Те, що в автора, те й у друга (у вибраних теках).
    for f in plan.files.iter().filter(|f| plan.selection.picks(&f.path)) {
        assert!(g2.join(&f.path).exists(), "у друга немає {}", f.path);
    }

    // Автор додав русифікатор і ввімкнув його в грі.
    put(&game, "resourcepacks/Russifier.zip", b"russifier");
    let opts = std::fs::read_to_string(game.join("options.txt")).unwrap_or_default();
    let packs_line = opts.lines().find(|l| l.starts_with("resourcePacks:")).map(str::to_string).unwrap_or_else(|| "resourcePacks:[\"vanilla\"]".into());
    let mut list: Vec<String> = serde_json::from_str(packs_line.trim_start_matches("resourcePacks:")).unwrap();
    list.push("file/Russifier.zip".into());
    let opts = if opts.contains("resourcePacks:") { opts.replace(&packs_line, &format!("resourcePacks:{}", serde_json::to_string(&list).unwrap())) } else { format!("{opts}resourcePacks:{}\n", serde_json::to_string(&list).unwrap()) };
    std::fs::write(game.join("options.txt"), opts).unwrap();
    let t0 = Instant::now();
    let p2 = packs::publish(&author, &inst, &host.token, "public", "", Some(pid.clone()), plan.selection.clone(), true, &task).await.expect("v2");
    println!("  v{} за {:?}", p2["version"], t0.elapsed());

    let t0 = Instant::now();
    packs::update(&friend, &guest.token, &mut mine, "LiaTestHost", &ftask).await.expect("оновлення");
    println!("  друг оновився за {:?}", t0.elapsed());
    assert_eq!(std::fs::read(g2.join("resourcepacks/Russifier.zip")).unwrap(), b"russifier");
    let o2 = std::fs::read_to_string(g2.join("options.txt")).unwrap();
    assert!(o2.lines().any(|l| l.starts_with("resourcePacks:") && l.contains("file/Russifier.zip")), "русифікатор не ввімкнено");
    lia::call(&author.http, reqwest::Method::DELETE, &format!("/api/packs/{pid}"), Some(&host.token), None).await.expect("видалення");
}
