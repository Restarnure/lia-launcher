# ЛІЯ — лаунчер Minecraft

Український лаунчер Minecraft: Java Edition. Без реклами, без стеження й без посередників: гра, бібліотеки та Java беруться лише з офіційних серверів Mojang, а моди — з Modrinth.

## Що вміє

- **Будь-яка версія гри** — від b1.7.3 і 1.5.2 до 26.x і знімків. Перевірено справжнім запуском: 1.5.2, 1.6.4, 1.7.10, 1.8.9, 1.12.2, 1.16.5, 1.21.11, 26.3.
- **Модлоадери в один клік**: Fabric, Quilt, Forge (від 1.7.10 до 26.x), NeoForge (1.21.1 і 26.x).
- **Java ставиться сама**: офіційні рантайми Mojang (8, 17, 21, 25) під кожну версію гри.
- **Збірки** з окремими модами, світами й налаштуваннями (пам'ять, Java, аргументи JVM, розмір вікна).
- **Каталог Modrinth**: моди, модпаки, ресурспаки, шейдери. Фільтр під конкретну збірку, автоматичні залежності, оновлення встановлених модів, імпорт `.mrpack` (кнопкою або перетягуванням у вікно).
- **Акаунти**: Microsoft (ліцензія, скіни, плащі) і офлайн-нік. Токени шифруються ключем користувача Windows (DPAPI).
- **3D-перегляд скіна**, зміна скіна й плаща для Microsoft-акаунтів.
- **Shotam на головній**: живий онлайн і MOTD сервера, «Грати на Shotam» сам створює збірку потрібної версії з голосовим чатом і заходить на сервер.
- **Консоль гри** з рівнями, пошуком і підсвіткою; **розпізнавання падінь** (брак пам'яті, не та Java, несумісні моди, драйвер відеокарти…) з поясненням людською мовою.
- **Швидкий вхід** у світ чи на сервер (1.20+), світи й скріншоти збірки, видалення в кошик (нічого не зникає безповоротно).
- **Перехід з TLauncher за хвилину**: світи, ресурспаки, шейдери, моди, налаштування й сервери з будь-якої теки `.minecraft` у нову збірку (оригінали не змінюються). Остання версія зі старого лаунчера підставляється сама.
- **Shotam у списку серверів** кожної нової збірки (лише коли списку ще немає; вимикається разом з карткою на головній).
- **Легкий**: інсталятор 3,3 МБ, застосунок 8 МБ, майже не їсть пам'ять, поки йде гра.
- **Працює без інтернету**, якщо гру вже завантажено. Гра, запущена до перезапуску лаунчера, підхоплюється — ту саму збірку не запустити двічі.
- Українська та англійська мови інтерфейсу.

### Що перевірено справжнім запуском

| | |
|---|---|
| Vanilla | 1.5.2, 1.6.4, 1.7.10, 1.8.9, 1.12.2, 1.16.5, 1.21.11, 26.3, 26.4-snapshot-5 |
| Fabric / Quilt | Fabric 1.21.11 (+Sodium, Mod Menu, Fabric API), Quilt 1.21.1 |
| Forge | 1.7.10, 1.12.2, 1.20.1, 26.3 |
| NeoForge | 1.21.1, 26.3 |
| Модпак | Fabulously Optimized 12.3 (149 модів) |
| Сервер | Shotam: офлайн-нік, швидкий вхід, голосовий чат |

## Встановлення

Інсталятор після збірки: `src-tauri/target/release/bundle/nsis/ЛІЯ_1.0.0_x64-setup.exe`. Ставиться для поточного користувача, прав адміністратора не потребує.

Дані лежать у `%APPDATA%\.lia` (теку гри можна змінити в налаштуваннях):

```
.lia/
  launcher.json        налаштування
  accounts.dat         акаунти (зашифровано DPAPI)
  instances/<id>/      збірки: instance.json + .minecraft/ (моди, світи…)
  versions/ libraries/ assets/   спільні файли гри (як у .minecraft)
  runtimes/            Java від Mojang
```

## Розробка

Потрібні: Node.js 20+, Rust (stable), Visual Studio Build Tools з «Desktop development with C++», WebView2 (є в Windows 11).

```bash
npm install
npm run tauri dev
```

Збірка інсталятора:

```bash
npm run tauri build
```

Тести ядра (без мережі):

```bash
cd src-tauri && cargo test --lib
```

Справжній запуск гри (качає ~1 ГБ у `%TEMP%\lia-e2e`, вікна гри на кілька секунд з'являться на екрані):

```bash
cd src-tauri && LIA_E2E="26.3,1.12.2,fabric:1.21.11,forge:1.20.1,neoforge:1.21.1" cargo test --test e2e -- --ignored --nocapture
```

Змінні середовища: `LIA_HOME` — тека налаштувань, `LIA_DATA` — тека даних гри (для тестів і портативного режиму).

## Вхід Microsoft

Mojang пускає до Minecraft API лише застосунки Azure, які вона схвалила. Тому вхід Microsoft вмикається власним Client ID:

1. [portal.azure.com](https://portal.azure.com) → App registrations → New registration → «Personal Microsoft accounts only».
2. Authentication → Add a platform → Mobile and desktop applications → redirect URI `https://login.microsoftonline.com/common/oauth2/nativeclient`. Увімкнути «Allow public client flows».
3. Подати Client ID на схвалення: https://aka.ms/mce-reviewappid
4. Після схвалення вписати ID у «Налаштування → Вхід Microsoft» або вшити в збірку: `LIA_MS_CLIENT_ID=<id> npm run tauri build`.

Поки ID не схвалено, Microsoft повертає «Invalid app registration» — лаунчер пояснить це повідомленням.

## Shotam

Закріплений сервер описано в `src-tauri/src/servers.rs` (`builtin()`): адреса, версія гри, модлоадер, моди з Modrinth. Щоб змінювати це без нового релізу лаунчера, достатньо викласти на сайті `https://shotam.today/launcher.json`:

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

Коли сервер перейде на нову версію, збірка «Shotam» у гравців сама оновиться до неї при наступному «Грати на Shotam».

## Архітектура

- `src-tauri/src/` — ядро на Rust (Tauri 2): `mojang/` (версії, правила, бібліотеки, ресурси, Java), `loaders.rs`, `launch.rs` (аргументи, запуск, лог), `modrinth.rs`, `content.rs`, `accounts.rs`, `servers.rs`, `app.rs` (команди для інтерфейсу).
- `src/` — інтерфейс на React + TypeScript: `pages/`, `components/`, `lib/` (стан, переклади, піксельні блоки, MOTD).
