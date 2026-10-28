use super::rules::{self, Env, Rule};
use crate::util::maven_path;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct VersionJson {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherits_from: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_arguments: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Arguments>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_index: Option<AssetIndexRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<HashMap<String, Artifact>>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java_version: Option<JavaVersionRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logging: Option<Logging>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_time: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<Arg>,
    #[serde(default)]
    pub jvm: Vec<Arg>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(untagged)]
pub enum Arg {
    Plain(String),
    Ruled { rules: Vec<Rule>, value: ArgValue },
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(untagged)]
pub enum ArgValue {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AssetIndexRef {
    pub id: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub total_size: Option<u64>,
    pub url: String,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct Artifact {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default)]
    pub url: String,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersionRef {
    #[serde(default)]
    pub component: Option<String>,
    #[serde(default)]
    pub major_version: Option<u32>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Logging {
    #[serde(default)]
    pub client: Option<LoggingClient>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct LoggingClient {
    pub argument: String,
    pub file: LogFile,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct LogFile {
    pub id: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    pub url: String,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct Library {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<LibDownloads>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub natives: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract: Option<Extract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<Rule>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct LibDownloads {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<Artifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifiers: Option<HashMap<String, Artifact>>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct Extract {
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// Файл бібліотеки відносно теки `libraries/`. `url == None` — файл має вже бути на диску
/// (його створює інсталятор Forge/NeoForge).
#[derive(Clone, Debug, PartialEq)]
pub struct LibFile {
    pub path: String,
    pub url: Option<String>,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

const MOJANG_LIBS: &str = "https://libraries.minecraft.net/";

impl Library {
    pub fn allowed(&self, env: &Env) -> bool {
        rules::allowed(self.rules.as_deref().unwrap_or(&[]), env)
    }

    fn from_artifact(&self, a: &Artifact, coord: &str) -> Option<LibFile> {
        let path = a.path.clone().filter(|p| !p.is_empty()).or_else(|| maven_path(coord))?;
        Some(LibFile { path, url: Some(a.url.clone()).filter(|u| !u.is_empty()), sha1: a.sha1.clone(), size: a.size })
    }

    fn from_maven(&self, coord: &str) -> Option<LibFile> {
        let path = maven_path(coord)?;
        let mut base = self.url.clone().filter(|u| !u.is_empty()).unwrap_or_else(|| MOJANG_LIBS.into());
        if !base.ends_with('/') {
            base.push('/');
        }
        Some(LibFile { url: Some(format!("{base}{path}")), path, sha1: self.sha1.clone(), size: self.size })
    }

    /// Jar для classpath. Нові нативні бібліотеки (`...:natives-windows`) теж ідуть сюди.
    pub fn artifact(&self) -> Option<LibFile> {
        match &self.downloads {
            Some(d) => d.artifact.as_ref().and_then(|a| self.from_artifact(a, &self.name)),
            None if self.natives.is_some() => None,
            None => self.from_maven(&self.name),
        }
    }

    /// Старі нативні бібліотеки (поле `natives`), які треба розпакувати перед запуском.
    pub fn native(&self, env: &Env) -> Option<LibFile> {
        let classifier = self.natives.as_ref()?.get(env.os_name)?.replace("${arch}", env.arch_bits());
        let coord = format!("{}:{classifier}", self.name);
        if let Some(a) = self.downloads.as_ref().and_then(|d| d.classifiers.as_ref()).and_then(|c| c.get(&classifier)) {
            return self.from_artifact(a, &coord);
        }
        self.from_maven(&coord)
    }

    /// Ключ без версії: дочірній профіль (Fabric, Forge) перекриває бібліотеку батька.
    fn dedupe_key(&self) -> String {
        let mut parts = self.name.split('@').next().unwrap_or("").split(':');
        let group = parts.next().unwrap_or("");
        let artifact = parts.next().unwrap_or("");
        let _version = parts.next();
        let classifier = parts.next().unwrap_or("");
        format!("{group}:{artifact}:{classifier}")
    }
}

impl VersionJson {
    /// Злиття дочірнього профілю (Fabric/Forge) з батьківським (ванільним).
    pub fn merge_onto(self, parent: VersionJson) -> VersionJson {
        let jar = self.jar.clone().or(parent.jar.clone()).or_else(|| Some(parent.id.clone()));
        let arguments = match (parent.arguments, self.arguments) {
            (None, None) => None,
            (p, c) => {
                let p = p.unwrap_or_default();
                let c = c.unwrap_or_default();
                Some(Arguments { game: [p.game, c.game].concat(), jvm: [p.jvm, c.jvm].concat() })
            }
        };
        // Лише дочірній перекриває батька. Власні дублікати батька означають варіанти під різні ОС
        // (lwjgl 3.2.1 для macOS і 3.2.2 для решти в 1.14–1.18) або окремий запис natives (1.13).
        let child: HashSet<String> = self.libraries.iter().map(Library::dedupe_key).collect();
        let libraries = self.libraries.into_iter().chain(parent.libraries.into_iter().filter(|l| !child.contains(&l.dedupe_key()))).collect();
        VersionJson {
            id: self.id,
            inherits_from: parent.inherits_from,
            kind: self.kind.or(parent.kind),
            main_class: self.main_class.or(parent.main_class),
            minecraft_arguments: self.minecraft_arguments.or(parent.minecraft_arguments),
            arguments,
            asset_index: self.asset_index.or(parent.asset_index),
            assets: self.assets.or(parent.assets),
            downloads: self.downloads.or(parent.downloads),
            libraries,
            java_version: self.java_version.or(parent.java_version),
            logging: self.logging.or(parent.logging),
            jar,
            release_time: self.release_time.or(parent.release_time),
        }
    }

    pub fn java_major(&self) -> u32 {
        self.java_version.as_ref().and_then(|j| j.major_version).unwrap_or(8)
    }

    pub fn java_component(&self) -> String {
        self.java_version.as_ref().and_then(|j| j.component.clone()).unwrap_or_else(|| "jre-legacy".into())
    }

    /// Чи підтримує версія аргумент швидкого входу на сервер (1.20+).
    pub fn supports_quick_play(&self) -> bool {
        self.arguments.as_ref().is_some_and(|a| {
            a.game.iter().any(|g| matches!(g, Arg::Ruled { value: ArgValue::Many(v), .. } if v.iter().any(|s| s == "--quickPlayMultiplayer")))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win() -> Env {
        Env { os_name: "windows", os_arch: "x86_64", os_version: "10.0".into(), features: Default::default() }
    }

    #[test]
    fn legacy_natives() {
        let lib: Library = serde_json::from_str(
            r#"{"name":"org.lwjgl.lwjgl:lwjgl-platform:2.9.4-nightly-20150209",
                "downloads":{"classifiers":{"natives-windows":{"path":"org/lwjgl/lwjgl/lwjgl-platform/2.9.4-nightly-20150209/lwjgl-platform-2.9.4-nightly-20150209-natives-windows.jar","sha1":"abc","size":10,"url":"https://libraries.minecraft.net/x.jar"}}},
                "natives":{"windows":"natives-windows"}}"#,
        )
        .unwrap();
        assert!(lib.artifact().is_none());
        let n = lib.native(&win()).unwrap();
        assert!(n.path.ends_with("natives-windows.jar"));
        assert_eq!(n.sha1.as_deref(), Some("abc"));
    }

    #[test]
    fn arch_placeholder() {
        let lib: Library = serde_json::from_str(
            r#"{"name":"tv.twitch:twitch-platform:5.16","natives":{"windows":"natives-windows-${arch}"}}"#,
        )
        .unwrap();
        let n = lib.native(&win()).unwrap();
        assert_eq!(n.path, "tv/twitch/twitch-platform/5.16/twitch-platform-5.16-natives-windows-64.jar");
        assert_eq!(n.url.unwrap(), "https://libraries.minecraft.net/tv/twitch/twitch-platform/5.16/twitch-platform-5.16-natives-windows-64.jar");
    }

    #[test]
    fn maven_style() {
        let lib: Library = serde_json::from_str(
            r#"{"name":"net.fabricmc:fabric-loader:0.16.9","url":"https://maven.fabricmc.net/","sha1":"s","size":5}"#,
        )
        .unwrap();
        let a = lib.artifact().unwrap();
        assert_eq!(a.url.unwrap(), "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar");
        assert_eq!(a.size, Some(5));
    }

    #[test]
    fn forge_generated_artifact_has_no_url() {
        let lib: Library = serde_json::from_str(
            r#"{"name":"net.minecraftforge:forge:1.20.1-47.2.0:client","downloads":{"artifact":{"path":"net/minecraftforge/forge/1.20.1-47.2.0/forge-1.20.1-47.2.0-client.jar","url":"","sha1":"x","size":1}}}"#,
        )
        .unwrap();
        assert_eq!(lib.artifact().unwrap().url, None);
    }

    #[test]
    fn merge_child_overrides() {
        let parent: VersionJson = serde_json::from_str(
            r#"{"id":"1.20.1","mainClass":"net.minecraft.client.main.Main","assets":"5",
                "arguments":{"game":["--username","${auth_player_name}"],"jvm":["-cp","${classpath}"]},
                "libraries":[{"name":"org.ow2.asm:asm:9.3"},{"name":"org.lwjgl:lwjgl:3.3.1"},{"name":"org.lwjgl:lwjgl:3.3.1:natives-windows"}]}"#,
        )
        .unwrap();
        let child: VersionJson = serde_json::from_str(
            r#"{"id":"fabric-loader-0.16.9-1.20.1","inheritsFrom":"1.20.1","mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient",
                "arguments":{"game":[],"jvm":["-DFabricMcEmu= net.minecraft.client.main.Main "]},
                "libraries":[{"name":"org.ow2.asm:asm:9.7","url":"https://maven.fabricmc.net/"}]}"#,
        )
        .unwrap();
        let m = child.merge_onto(parent);
        assert_eq!(m.id, "fabric-loader-0.16.9-1.20.1");
        assert_eq!(m.main_class.unwrap(), "net.fabricmc.loader.impl.launch.knot.KnotClient");
        assert_eq!(m.jar.unwrap(), "1.20.1");
        assert_eq!(m.assets.unwrap(), "5");
        let names: Vec<_> = m.libraries.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, vec!["org.ow2.asm:asm:9.7", "org.lwjgl:lwjgl:3.3.1", "org.lwjgl:lwjgl:3.3.1:natives-windows"]);
        let a = m.arguments.unwrap();
        assert_eq!(a.game.len(), 2);
        assert_eq!(a.jvm.len(), 3);
    }

    /// 1.14–1.18 перелічують lwjgl 3.2.1 (лише macOS) перед 3.2.2, а 1.13 має jar і natives окремими записами.
    #[test]
    fn merge_keeps_parent_variants() {
        let parent: VersionJson = serde_json::from_str(
            r#"{"id":"1.16.5","libraries":[
                {"name":"org.lwjgl:lwjgl:3.2.1","rules":[{"action":"allow","os":{"name":"osx"}}]},
                {"name":"org.lwjgl:lwjgl:3.2.2","rules":[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]},
                {"name":"org.lwjgl:lwjgl:3.1.6","natives":{"windows":"natives-windows"}}]}"#,
        )
        .unwrap();
        let child: VersionJson = serde_json::from_str(r#"{"id":"fabric","inheritsFrom":"1.16.5","libraries":[{"name":"net.fabricmc:fabric-loader:0.16.9"}]}"#).unwrap();
        let m = child.merge_onto(parent);
        let names: Vec<_> = m.libraries.iter().filter(|l| l.allowed(&win())).map(|l| l.name.as_str()).collect();
        assert_eq!(names, vec!["net.fabricmc:fabric-loader:0.16.9", "org.lwjgl:lwjgl:3.2.2", "org.lwjgl:lwjgl:3.1.6"]);
    }
}
