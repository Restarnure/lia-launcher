//! Листи ЛІЯ ID через Resend (https://resend.com). Без ключа RESEND_API_KEY пошта вимкнена:
//! акаунти не потребують підтвердження, а відновлення пароля недоступне.
use crate::App;
use serde_json::json;

pub fn enabled() -> bool {
    std::env::var("RESEND_API_KEY").is_ok_and(|k| !k.is_empty()) || log_only()
}

/// LIA_MAIL_LOG=1 — для тестів: лист друкується в лог замість надсилання.
fn log_only() -> bool {
    std::env::var("LIA_MAIL_LOG").is_ok()
}

fn from() -> String {
    std::env::var("LIA_MAIL_FROM").unwrap_or_else(|_| "ЛІЯ <noreply@lia.starnure.dev>".into())
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Надсилає лист у фоні: відповідь API не чекає на поштовий сервіс.
pub fn send(app: &App, to: &str, subject: &str, lines: &[String]) {
    if log_only() {
        eprintln!("MAIL to={to} subject={subject} | {}", lines.join(" | "));
        return;
    }
    let Ok(key) = std::env::var("RESEND_API_KEY") else { return };
    let text = lines.join("\n\n");
    let html = format!(
        "<div style=\"font-family:Segoe UI,Arial,sans-serif;font-size:15px;line-height:1.5;color:#1b1f2a\">{}<p style=\"color:#8a93a5;font-size:12px\">Цей лист надіслала ЛІЯ, лаунчер Minecraft. Якщо це були не ви, змініть пароль у лаунчері.</p></div>",
        lines.iter().map(|l| format!("<p>{}</p>", esc(l))).collect::<String>()
    );
    let body = json!({ "from": from(), "to": [to], "subject": subject, "text": text, "html": html });
    let http = app.http.clone();
    tokio::spawn(async move {
        match http.post("https://api.resend.com/emails").bearer_auth(key).json(&body).send().await {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => eprintln!("пошта: HTTP {} {}", r.status(), r.text().await.unwrap_or_default()),
            Err(e) => eprintln!("пошта: {e}"),
        }
    });
}

/// Лист з кодом підтвердження.
pub fn code(app: &App, to: &str, subject: &str, intro: &str, code: &str) {
    send(app, to, subject, &[intro.to_string(), format!("Код: {code}"), "Код дійсний 15 хвилин. Нікому його не передавайте.".into()]);
}
