use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Rule {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<OsRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<HashMap<String, bool>>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct OsRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
}

/// Середовище, проти якого перевіряються правила version JSON.
#[derive(Clone, Debug)]
pub struct Env {
    pub os_name: &'static str,
    pub os_arch: &'static str,
    pub os_version: String,
    pub features: HashSet<String>,
}

impl Env {
    pub fn current() -> Env {
        let os_name = match std::env::consts::OS {
            "windows" => "windows",
            "macos" => "osx",
            _ => "linux",
        };
        let os_arch = match std::env::consts::ARCH {
            "x86" => "x86",
            "aarch64" => "arm64",
            _ => "x86_64",
        };
        // Java повідомляє "10.0" і для Windows 10, і для 11 — правила Mojang розраховані саме на це.
        let os_version = if os_name == "windows" { "10.0".to_string() } else { sysinfo::System::kernel_version().unwrap_or_default() };
        Env { os_name, os_arch, os_version, features: HashSet::new() }
    }

    pub fn with_feature(mut self, f: &str, on: bool) -> Env {
        if on {
            self.features.insert(f.to_string());
        }
        self
    }

    /// Підстановка `${arch}` у класифікаторах нативних бібліотек старих версій.
    pub fn arch_bits(&self) -> &'static str {
        if self.os_arch == "x86" {
            "32"
        } else {
            "64"
        }
    }
}

/// Порожній список — дозволено. Інакше перемагає останнє правило, що збіглося.
pub fn allowed(rules: &[Rule], env: &Env) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allow = false;
    for r in rules {
        if matches(r, env) {
            allow = r.action == "allow";
        }
    }
    allow
}

fn matches(r: &Rule, env: &Env) -> bool {
    if let Some(os) = &r.os {
        if os.name.as_deref().is_some_and(|n| n != env.os_name) {
            return false;
        }
        if os.arch.as_deref().is_some_and(|a| a != env.os_arch) {
            return false;
        }
        if let Some(v) = &os.version {
            match regex_lite::Regex::new(v) {
                Ok(re) if re.is_match(&env.os_version) => {}
                _ => return false,
            }
        }
    }
    if let Some(features) = &r.features {
        for (k, want) in features {
            if env.features.contains(k) != *want {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(json: &str) -> Vec<Rule> {
        serde_json::from_str(json).unwrap()
    }

    fn win() -> Env {
        Env { os_name: "windows", os_arch: "x86_64", os_version: "10.0".into(), features: HashSet::new() }
    }

    #[test]
    fn empty_allows() {
        assert!(allowed(&[], &win()));
    }

    #[test]
    fn os_name() {
        let r = rules(r#"[{"action":"allow","os":{"name":"osx"}}]"#);
        assert!(!allowed(&r, &win()));
        let r = rules(r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]"#);
        assert!(allowed(&r, &win()));
    }

    #[test]
    fn os_version_regex() {
        let r = rules(r#"[{"action":"allow","os":{"name":"windows","version":"^10\\."}}]"#);
        assert!(allowed(&r, &win()));
        let r = rules(r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx","version":"^10\\.5\\.\\d$"}}]"#);
        assert!(allowed(&r, &win()));
    }

    #[test]
    fn arch() {
        let r = rules(r#"[{"action":"allow","os":{"arch":"x86"}}]"#);
        assert!(!allowed(&r, &win()));
        let mut e = win();
        e.os_arch = "x86";
        assert!(allowed(&r, &e));
    }

    #[test]
    fn features() {
        let r = rules(r#"[{"action":"allow","features":{"has_custom_resolution":true}}]"#);
        assert!(!allowed(&r, &win()));
        assert!(allowed(&r, &win().with_feature("has_custom_resolution", true)));
    }
}
