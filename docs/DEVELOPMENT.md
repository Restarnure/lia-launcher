# Розробка

Потрібні Node.js 20+, Rust stable, Visual Studio Build Tools з «Desktop development with C++» і WebView2.

```bash
npm install
npm run tauri dev
```

Підписаний інсталятор (потрібен ключ, див. «Випуск оновлення»):

```bash
node scripts/release.mjs 1.0.0
```

Без ключа, автооновлення в такій збірці не працюватимуть:

```bash
npm run tauri build -- --config "{\"bundle\":{\"createUpdaterArtifacts\":false}}"
```

Тести без мережі:

```bash
cd src-tauri && cargo test --lib
```

E2E із запуском гри, качає близько 1 ГБ у `%TEMP%\lia-e2e`:

```bash
cd src-tauri && LIA_E2E="26.3,1.12.2,fabric:1.21.11,forge:1.20.1,neoforge:1.21.1" cargo test --test e2e -- --ignored --nocapture
```

`LIA_HOME` і `LIA_DATA` перевизначають теки налаштувань і даних гри, це для тестів і портативного режиму.

## Випуск оновлення

```bash
node scripts/release.mjs 1.0.1 "Що нового (markdown)" --publish
```

Піднімає версію, збирає підписаний інсталятор і публікує реліз на GitHub. Без `--publish` тільки збирає `release/<версія>/`.

Лаунчер перевіряє оновлення за адресою `https://github.com/Restarnure/lia-launcher/releases/latest/download/latest.json`.

Приватний ключ лежить у `%USERPROFILE%\.tauri\lia-updater.key` і в репозиторій не потрапляє, публічний прописаний у `tauri.conf.json`. Без приватного ключа встановлені лаунчери не приймуть жодного оновлення, тож тримай копію.

## Вхід Microsoft

Client ID схваленого застосунку Azure вшитий у `BUILT_IN` (`src-tauri/src/settings.rs`). Як отримати свій з нуля: [MICROSOFT.md](MICROSOFT.md).

## Закріплений сервер

Рекламне місце на головній, зараз там Shotam. Налаштування в `src-tauri/src/servers.rs` (`builtin()`): адреса, версія гри, модлоадер, моди з Modrinth. Без релізу їх перевизначає `https://shotam.today/launcher.json` у тому ж форматі:

```json
{
  "id": "shotam",
  "name": "Shotam",
  "address": "shotam.today",
  "port": 25565,
  "mcVersion": "1.21.11",
  "loader": "fabric",
  "mods": ["simple-voice-chat"],
  "website": "https://shotam.today",
  "descriptionUk": "Український сервер виживання…",
  "descriptionEn": "Ukrainian survival server…"
}
```

Коли сервер змінить версію гри, збірка гравця оновиться при наступному натисканні кнопки на картці.

Інший сервер так не поставити: адреса файлу і перевірка `id == "shotam"` у `valid()` зашиті в код, потрібен реліз.

## Сервер ЛІЇ (ЛІЯ ID, друзі, ретранслятор)

Код у `server/` (Rust, axum, SQLite). Працює на Oracle (`/srv/lia`, docker compose з `server/deploy/`):

- `https://lia.starnure.dev` через Cloudflare Tunnel `lia`: API лаунчера (`/api/…`), Yggdrasil для authlib-injector (`/yggdrasil`), текстури скінів.
- `*.play.lia.starnure.dev:25570` напряму: ретранслятор LAN-світів. Код світу береться з адреси в handshake Minecraft, службові з'єднання лаунчера починаються з байтів `00 LIA`.
- У VCN Oracle і в `DOCKER-USER` (`/etc/iptables/rules.v4`) відкрито TCP 25570.

Ключ підпису текстур `/srv/lia/data/signing.pem` створюється один раз (`openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:4096`), а база й файли лежать у `/srv/lia/data`.

Оновити сервер: скопіювати `server/{Cargo.toml,Cargo.lock,src,Dockerfile}` у `/srv/lia/server` і `docker compose up -d --build`.

Старіших лаунчерів сервер не пускає: API і Yggdrasil відповідають 426 з проханням оновитися, ретранслятор не відкриває їм світ. Мінімальна версія за замовчуванням прописана в `server/src/main.rs`, а без нової збірки сервера її можна підняти рядком `LIA_MIN_LAUNCHER=1.2.0` у `/srv/lia/lia.env` і перезапуском контейнера. Робіть це лише тоді, коли потрібна версія вже опублікована, інакше ЛІЯ ID перестане працювати в усіх.

ЛІЯ ID, акаунт лаунчера (токен без профілю, `/api/…`), має підтвердження пошти, відновлення й зміну пароля, список пристроїв, блокування після 8 невдалих входів і видалення акаунта (`server/src/account.rs`). Листи надсилає Resend: ключ `RESEND_API_KEY` у `/srv/lia/lia.env` (без нього пошта вимкнена, а акаунти не потребують підтвердження). Для тестів `LIA_MAIL_LOG=1` друкує листи в лог. Гра входить ігровим профілем: офлайн-нік закріплюється за ЛІЯ ID (`/api/game/session`), ліцензія входить лише кнопкою «З друзями», через join на сервері Mojang з serverId від ЛІЇ (`/api/game/challenge`), без передачі токена Mojang. Ігрові токени не відкривають `/api/…`. Невідомого гравця `hasJoined` перевіряє в Mojang і перепідписує профіль ключем ЛІЇ, тому ліцензія без ЛІЇ заходить у світи гравців ЛІЇ.

Така гра запускається з `-javaagent:authlib-injector.jar=https://lia.starnure.dev/yggdrasil` і `-Dhttp.agent=LIA-Launcher/…` (Cloudflare відсікає голий `Java/1.8…`).

Перевірки:

```bash
cd server && cargo test
python server/e2e.py
```

`e2e.py` проти локального сервера (`LIA_HTTP=127.0.0.1:18080 LIA_PLAY_DOMAIN=play.localhost`) або бойового, деталі в шапці файлу.

```bash
cd src-tauri && cargo test --test lia -- --ignored --nocapture --test-threads=1
```

Експорт і встановлення збірки, а також світ через ретранслятор: Paper в online-mode з authlib-injector і бот, що заходить як гравець ЛІЯ ID. Змінні описані в `tests/lia.rs`. `LIA_E2E_LIA=1` у `tests/e2e.rs` запускає гру з ЛІЯ ID.

Редактор скінів: `node scripts/skinmap.check.ts` перевіряє розкладку текстури й дзеркало.

## Структура

- `src-tauri/src/`: ядро на Rust (Tauri 2): `mojang/` (версії, бібліотеки, ресурси, Java), `loaders.rs`, `launch.rs`, `modrinth.rs`, `content.rs`, `accounts.rs`, `skinlib.rs`, `servers.rs`, `app.rs` (команди для інтерфейсу).
- `src/`: інтерфейс на React і TypeScript: `pages/`, `components/`, `lib/`.
- `scripts/release.mjs`: випуск оновлення.
- `server/`: сервер ЛІЇ (ЛІЯ ID, друзі, збірки, ретранслятор). Його клієнт у лаунчері: `src-tauri/src/lia.rs` і `src/lib/lia.ts`.
