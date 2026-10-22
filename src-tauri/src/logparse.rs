use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct LogLine {
    /// Мітка часу в мс (0 — невідомо).
    pub t: i64,
    /// INFO / WARN / ERROR / FATAL / DEBUG / TRACE, або порожньо для простого виводу.
    pub level: String,
    pub thread: String,
    pub text: String,
}

/// Розбирає вивід гри. Конфіг логування Mojang пише події log4j у форматі XML
/// (саме так їх читає й офіційний лаунчер); усе інше — звичайні рядки.
#[derive(Default)]
pub struct Parser {
    event: Option<String>,
}

impl Parser {
    pub fn feed(&mut self, line: &str, stderr: bool) -> Option<LogLine> {
        if let Some(buf) = &mut self.event {
            buf.push_str(line);
            buf.push('\n');
            if line.contains("</log4j:Event>") {
                let ev = self.event.take().unwrap();
                return Some(parse_event(&ev));
            }
            return None;
        }
        if line.trim_start().starts_with("<log4j:Event") {
            if line.contains("</log4j:Event>") {
                return Some(parse_event(line));
            }
            self.event = Some(format!("{line}\n"));
            return None;
        }
        Some(LogLine { t: 0, level: guess_level(line, stderr), thread: String::new(), text: line.to_string() })
    }
}

fn guess_level(line: &str, stderr: bool) -> String {
    let l = if line.contains("/ERROR]") || line.contains("[ERROR]") || line.contains("/FATAL]") {
        "ERROR"
    } else if line.contains("/WARN]") || line.contains("[WARN]") {
        "WARN"
    } else if line.contains("/DEBUG]") {
        "DEBUG"
    } else if line.contains("/INFO]") || line.contains("[INFO]") {
        "INFO"
    } else if stderr && (line.contains("Exception") || line.contains("Error") || line.starts_with("\tat ")) {
        "ERROR"
    } else if stderr {
        "WARN"
    } else {
        ""
    };
    l.to_string()
}

fn attr(tag: &str, name: &str) -> String {
    let key = format!("{name}=\"");
    tag.find(&key)
        .and_then(|i| {
            let rest = &tag[i + key.len()..];
            rest.find('"').map(|j| unescape(&rest[..j]))
        })
        .unwrap_or_default()
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn element(ev: &str, name: &str) -> Option<String> {
    let open = format!("<log4j:{name}>");
    let close = format!("</log4j:{name}>");
    let start = ev.find(&open)? + open.len();
    let end = ev[start..].rfind(&close)? + start;
    let inner = ev[start..end].trim();
    Some(match inner.strip_prefix("<![CDATA[").and_then(|s| s.strip_suffix("]]>")) {
        Some(cdata) => cdata.replace("]]]]><![CDATA[>", "]]>"),
        None => unescape(inner),
    })
}

fn parse_event(ev: &str) -> LogLine {
    let tag_end = ev.find('>').unwrap_or(ev.len());
    let tag = &ev[..tag_end];
    let mut text = element(ev, "Message").unwrap_or_default();
    if let Some(thr) = element(ev, "Throwable") {
        if !thr.trim().is_empty() {
            text.push('\n');
            text.push_str(thr.trim_end());
        }
    }
    LogLine {
        t: attr(tag, "timestamp").parse().unwrap_or(0),
        level: attr(tag, "level"),
        thread: attr(tag, "thread"),
        text: text.trim_end().to_string(),
    }
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CrashInfo {
    /// Ключ для перекладу: oom, java_version, gpu_driver, mod_incompatible, mixin, ...
    pub kind: String,
    pub detail: Option<String>,
    pub report_path: Option<String>,
}

/// Підказка, чому впала гра, за логом і звітом про падіння.
pub fn analyze(log: &str, report: Option<&str>) -> CrashInfo {
    let all = format!("{log}\n{}", report.unwrap_or(""));
    let find_line = |needle: &str| all.lines().find(|l| l.contains(needle)).map(|l| l.trim().to_string());
    let has = |needles: &[&str]| needles.iter().any(|n| all.contains(n));
    let mk = |kind: &str, detail: Option<String>| CrashInfo { kind: kind.into(), detail, report_path: None };

    if has(&["java.lang.OutOfMemoryError"]) {
        return mk("oom", find_line("OutOfMemoryError"));
    }
    if has(&["Could not reserve enough space", "Invalid maximum heap size", "Initial heap size set to a larger value"]) {
        return mk("heap_reserve", find_line("heap"));
    }
    if has(&["Unrecognized VM option", "Unrecognized option:", "Error occurred during initialization of VM", "Could not create the Java Virtual Machine"]) {
        let line = find_line("Unrecognized").or_else(|| find_line("Error occurred during initialization"));
        return mk("jvm_args", line);
    }
    if has(&["UnsupportedClassVersionError", "compiled by a more recent version of the Java Runtime"]) {
        return mk("java_version", find_line("class file version").or_else(|| find_line("UnsupportedClassVersionError")));
    }
    if let Some(i) = all.find("Incompatible mods found!").or_else(|| all.find("incompatible mod set")) {
        let block: String = all[i..].lines().take(14).collect::<Vec<_>>().join("\n");
        return mk("mod_incompatible", Some(block));
    }
    if let Some(i) = all.find("Missing or unsupported mandatory dependencies") {
        let block: String = all[i..].lines().take(12).collect::<Vec<_>>().join("\n");
        return mk("mod_incompatible", Some(block));
    }
    if has(&["DuplicateModsFoundException", "Found duplicate mods", "Duplicate mods found", "duplicate mods"]) {
        return mk("duplicate_mods", find_line("uplicate"));
    }
    if has(&["MixinApplyError", "Mixin apply failed", "InvalidMixinException", "Mixin transformation of"]) {
        return mk("mixin", find_line("Mixin apply").or_else(|| find_line("mixin")));
    }
    if has(&["Pixel format not accelerated", "WGL_ARB_create_context", "GLFW error 65542", "GLFW error 65543", "Couldn't set pixel format", "No OpenGL context", "OpenGL 3.2"]) {
        return mk("gpu_driver", find_line("GLFW").or_else(|| find_line("pixel format")));
    }
    if has(&["EXCEPTION_ACCESS_VIOLATION"]) {
        let gpu = ["atio6axx", "atioglxx", "ig9icd", "igxelpicd", "ig7icd", "ig75icd", "nvoglv"];
        return if gpu.iter().any(|g| all.contains(g)) { mk("gpu_driver", find_line("Problematic frame")) } else { mk("native_crash", find_line("Problematic frame")) };
    }
    if has(&["java.lang.UnsatisfiedLinkError"]) {
        return mk("natives", find_line("UnsatisfiedLinkError"));
    }
    if has(&["NoClassDefFoundError", "ClassNotFoundException"]) {
        return mk("missing_class", find_line("NoClassDefFoundError").or_else(|| find_line("ClassNotFoundException")));
    }
    let detail = report
        .and_then(|r| r.lines().find(|l| l.starts_with("Description:")).map(|d| {
            let exc = r.lines().skip_while(|l| !l.starts_with("Description:")).skip(1).find(|l| !l.trim().is_empty()).unwrap_or("");
            format!("{}\n{}", d.trim(), exc.trim())
        }))
        .or_else(|| log.lines().rev().find(|l| l.contains("Exception") || l.contains("Error")).map(|l| l.trim().to_string()));
    mk("unknown", detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_events() {
        let mut p = Parser::default();
        assert_eq!(p.feed(r#"<log4j:Event logger="net.minecraft.client.Minecraft" timestamp="1700000000123" level="INFO" thread="Render thread">"#, false), None);
        assert_eq!(p.feed("  <log4j:Message><![CDATA[Setting user: Steve & <Alex>]]></log4j:Message>", false), None);
        let l = p.feed("</log4j:Event>", false).unwrap();
        assert_eq!(l.level, "INFO");
        assert_eq!(l.thread, "Render thread");
        assert_eq!(l.t, 1700000000123);
        assert_eq!(l.text, "Setting user: Steve & <Alex>");
    }

    #[test]
    fn throwable_and_multiline() {
        let mut p = Parser::default();
        let lines = [
            r#"<log4j:Event logger="x" timestamp="1" level="ERROR" thread="main">"#,
            "<log4j:Message><![CDATA[Boom",
            "second line]]></log4j:Message>",
            "<log4j:Throwable><![CDATA[java.lang.RuntimeException: x",
            "\tat a.b(C.java:1)",
            "]]></log4j:Throwable>",
            "</log4j:Event>",
        ];
        let mut out = None;
        for l in lines {
            out = p.feed(l, false).or(out);
        }
        let l = out.unwrap();
        assert_eq!(l.level, "ERROR");
        assert_eq!(l.text, "Boom\nsecond line\njava.lang.RuntimeException: x\n\tat a.b(C.java:1)");
    }

    #[test]
    fn plain_lines() {
        let mut p = Parser::default();
        let l = p.feed("[12:00:00] [main/WARN]: careful", false).unwrap();
        assert_eq!(l.level, "WARN");
        assert_eq!(p.feed("hello", false).unwrap().level, "");
        assert_eq!(p.feed("java.lang.IllegalStateException: x", true).unwrap().level, "ERROR");
    }

    #[test]
    fn crash_kinds() {
        assert_eq!(analyze("java.lang.OutOfMemoryError: Java heap space", None).kind, "oom");
        let c = analyze("Unrecognized VM option 'Foo'
Error: Could not create the Java Virtual Machine.", None);
        assert_eq!((c.kind.as_str(), c.detail.as_deref()), ("jvm_args", Some("Unrecognized VM option 'Foo'")));
        assert_eq!(
            analyze("Exception in thread main java.lang.UnsupportedClassVersionError: net/minecraft/client/main/Main has been compiled by a more recent version of the Java Runtime (class file version 65.0)", None).kind,
            "java_version"
        );
        let fabric = "Incompatible mods found!\n - Mod 'Sodium' (sodium) 0.6.0 requires version 0.16 or later of fabric-loader";
        let c = analyze(fabric, None);
        assert_eq!(c.kind, "mod_incompatible");
        assert!(c.detail.unwrap().contains("Sodium"));
        let report = "---- Minecraft Crash Report ----\nDescription: Rendering overlay\n\njava.lang.NullPointerException: oops\n";
        let c = analyze("", Some(report));
        assert_eq!(c.kind, "unknown");
        assert_eq!(c.detail.unwrap(), "Description: Rendering overlay\njava.lang.NullPointerException: oops");
    }
}
