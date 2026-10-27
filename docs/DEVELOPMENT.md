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

## Структура

- `src-tauri/src/`: ядро на Rust (Tauri 2): `mojang/` (версії, бібліотеки, ресурси, Java), `loaders.rs`, `launch.rs`, `modrinth.rs`, `content.rs`, `accounts.rs`, `skinlib.rs`, `servers.rs`, `app.rs` (команди для інтерфейсу).
- `src/`: інтерфейс на React і TypeScript: `pages/`, `components/`, `lib/`.
- `scripts/release.mjs`: випуск оновлення.
