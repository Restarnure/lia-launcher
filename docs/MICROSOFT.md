# Вхід Microsoft у ЛІЇ

Minecraft пускає до свого API лише ті програми, які Mojang схвалила. Тому лаунчеру потрібен
**власний застосунок Azure** (це безкоштовно) і **одноразове схвалення Mojang**. Після цього
ID вшивається в збірку — гравці просто тиснуть «Увійти через Microsoft», нічого не налаштовуючи.

Чужий Client ID (наприклад, офіційного лаунчера) використовувати не можна: це видавати свою
програму за чужу, і Microsoft такі входи блокує.

## 1. Реєстрація застосунку (≈10 хвилин)

1. Відкрийте <https://portal.azure.com> і увійдіть своїм акаунтом Microsoft
   (звичайний особистий акаунт підходить; якщо портал попросить створити каталог — погодьтесь, це безкоштовно).
2. У пошуку вгорі введіть **App registrations** → **New registration**.
3. Заповніть:
   - **Name:** `ЛІЯ` (або `LIA Launcher`) — цю назву гравці побачать на сторінці входу Microsoft.
   - **Supported account types:** *Personal Microsoft accounts only*.
   - **Redirect URI:** платформа *Public client/native (mobile & desktop)*, адреса
     `https://login.microsoftonline.com/common/oauth2/nativeclient`
4. Натисніть **Register**.
5. На сторінці застосунку скопіюйте **Application (client) ID** — це і є Client ID.
6. Розділ **Authentication** → внизу **Allow public client flows** → **Yes** → **Save**.

Секрет (client secret) і дозволи API **не потрібні**: ЛІЯ — публічний клієнт, а дозвіл
`XboxLive.signin` запитується під час входу.

## 2. Заявка на схвалення Mojang

1. Відкрийте форму <https://aka.ms/mce-reviewappid>.
2. Вкажіть Application (client) ID і Directory (tenant) ID зі сторінки застосунку, свій email,
   назву «ЛІЯ», коротко опишіть: *«Український лаунчер Minecraft: Java Edition з відкритим
   встановленням версій, модлоадерів і модів з Modrinth; вхід Microsoft для власників гри»*.
3. Відправте форму. Розгляд зазвичай займає від кількох днів до кількох тижнів; відповідь
   приходить на email.

Поки схвалення немає, сам вхід у Microsoft пройде, але на останньому кроці Minecraft відповість
403 — ЛІЯ покаже повідомлення «Mojang ще не схвалила цей Client ID».

## 3. Вшити ID у лаунчер

Будь-який з варіантів:

- вписати ID у `src-tauri/src/settings.rs` → `const BUILT_IN: &str = "<ваш ID>";` і випустити оновлення;
- або збирати зі змінною: `set LIA_MS_CLIENT_ID=<ваш ID>` перед `node scripts/release.mjs …`.

Для перевірки до випуску ID можна тимчасово вписати в **Налаштування → Для розробника**.

## Як працює вхід

1. ЛІЯ відкриває вікно з офіційною сторінкою входу Microsoft (OAuth 2.0 + PKCE, тенант `consumers`).
2. Після входу Microsoft перенаправляє на `…/nativeclient?code=…` — ЛІЯ перехоплює код, у браузер він не потрапляє.
3. Код обмінюється на токени → Xbox Live → XSTS → токен Minecraft → профіль гравця.
4. Токени зберігаються в `%APPDATA%\.lia\accounts.dat`, зашифровані ключем вашого користувача
   Windows (DPAPI), і тихо оновлюються перед запуском гри.
