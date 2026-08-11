//! Per-version JSON models.
//!
//! Each entry in the Mojang version manifest points at a per-version JSON
//! describing the client jar, libraries (with rules and natives), asset
//! index, and the JVM/game arguments. Only parsing lives here; rules
//! evaluation happens in the argument-resolution step (TASK-wpwuz).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// A parsed per-version JSON (`https://piston-meta.mojang.com/.../<id>.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionJson {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "mainClass", default)]
    pub main_class: Option<String>,
    /// Modern arguments block (present from ~1.13 onwards).
    #[serde(default)]
    pub arguments: Option<Arguments>,
    /// Legacy argument string (pre-1.13 versions).
    #[serde(rename = "minecraftArguments", default)]
    pub minecraft_arguments: Option<String>,
    #[serde(rename = "assetIndex", default)]
    pub asset_index: Option<AssetIndex>,
    #[serde(default)]
    pub assets: Option<String>,
    #[serde(rename = "javaVersion", default)]
    pub java_version: Option<JavaVersion>,
    #[serde(default)]
    pub downloads: Downloads,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub logging: Option<Logging>,
    #[serde(rename = "minimumLauncherVersion", default)]
    pub minimum_launcher_version: Option<i32>,
    pub time: String,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
}

/// The modern `arguments` block: `game` and `jvm` argument lists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<Argument>,
    #[serde(default)]
    pub jvm: Vec<Argument>,
}

/// A single argument: either a plain string or a rules-gated value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Argument {
    Plain(String),
    Ruled(RuledArgument),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuledArgument {
    pub rules: Vec<Rule>,
    pub value: ArgumentValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ArgumentValue {
    Single(String),
    Multi(Vec<String>),
}

/// A rule gating an argument or library on the current platform/features.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub action: String,
    #[serde(default)]
    pub os: Option<OsRule>,
    #[serde(default)]
    pub features: Option<BTreeMap<String, bool>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsRule {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetIndex {
    pub id: String,
    pub sha1: String,
    pub size: u64,
    #[serde(rename = "totalSize")]
    pub total_size: u64,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaVersion {
    pub component: String,
    #[serde(rename = "majorVersion")]
    pub major_version: i32,
}

/// Direct downloads from the version JSON (client/server jars, mappings).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Downloads {
    #[serde(default)]
    pub client: Option<ArtifactDownload>,
    #[serde(default)]
    pub client_mappings: Option<ArtifactDownload>,
    #[serde(default)]
    pub server: Option<ArtifactDownload>,
    #[serde(default)]
    pub server_mappings: Option<ArtifactDownload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactDownload {
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Library {
    pub name: String,
    /// Legacy maven base URL (e.g. loader profiles emit `url` instead of a
    /// `downloads` block). When present, it is used as the download host.
    #[serde(default)]
    pub url: Option<String>,
    /// Legacy checksum/size attached to the library itself (loader profiles
    /// may omit the `downloads` block but still verify by these).
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub downloads: Option<LibraryDownloads>,
    #[serde(default)]
    pub rules: Option<Vec<Rule>>,
    #[serde(default)]
    pub natives: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub extract: Option<Extract>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<ArtifactDownload>,
    #[serde(default)]
    pub classifiers: Option<BTreeMap<String, ArtifactDownload>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extract {
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// Logging config (usually a Log4j XML file passed via `-Dlog4j.configurationFile=`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Logging {
    pub client: LoggingClient,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingClient {
    pub argument: String,
    pub file: LoggingFile,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingFile {
    pub id: String,
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

/// Parse a per-version JSON document.
///
/// # Errors
///
/// Fails if the bytes are not valid JSON or do not match the version JSON schema.
pub fn parse(bytes: &[u8]) -> Result<VersionJson> {
    serde_json::from_slice(bytes).map_err(Error::from)
}

/// Fetch and parse a per-version JSON from `url`.
///
/// # Errors
///
/// Fails on network errors, non-2xx responses, or invalid JSON in the body.
pub async fn fetch(client: &reqwest::Client, url: &str) -> Result<VersionJson> {
    let bytes = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    parse(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODERN_JSON: &str = r#"{
        "id": "1.21.4",
        "type": "release",
        "mainClass": "net.minecraft.client.main.Main",
        "arguments": {
            "game": ["--username", "{\"name\":\"auth_player_name\"}",
                {"rules": [{"action": "allow", "features": {"has_custom_resolution": true}}],
                 "value": ["--width", "{\"name\":\"resolution_width\"}"]}],
            "jvm": ["-XX:+UseG1GC", {"rules": [{"action": "allow", "os": {"name": "osx"}}],
                 "value": "-XstartOnFirstThread"}]
        },
        "assetIndex": {
            "id": "25",
            "sha1": "1111111111111111111111111111111111111111",
            "size": 424242,
            "totalSize": 2679216554,
            "url": "https://piston-meta.mojang.com/v1/packages/1111111111111111111111111111111111111111/25.json"
        },
        "assets": "25",
        "javaVersion": {"component": "java-runtime-delta", "majorVersion": 21},
        "downloads": {
            "client": {"sha1": "2222222222222222222222222222222222222222", "size": 26953592, "url": "https://piston-meta.mojang.com/v1/objects/2222222222222222222222222222222222222222/client.jar"},
            "client_mappings": {"sha1": "3333333333333333333333333333333333333333", "size": 200921, "url": "https://piston-meta.mojang.com/v1/objects/3333333333333333333333333333333333333333/client.txt"},
            "server": {"sha1": "4444444444444444444444444444444444444444", "size": 54551252, "url": "https://piston-meta.mojang.com/v1/objects/4444444444444444444444444444444444444444/server.jar"}
        },
        "libraries": [
            {"name": "org.lwjgl:lwjgl:3.3.4",
             "downloads": {
                "artifact": {"sha1": "5555555555555555555555555555555555555555", "size": 12345, "url": "https://libraries.minecraft.net/org/lwjgl/lwjgl/3.3.4/lwjgl-3.3.4.jar"},
                "classifiers": {
                    "natives-windows": {"sha1": "6666666666666666666666666666666666666666", "size": 67890, "url": "https://libraries.minecraft.net/org/lwjgl/lwjgl/3.3.4/lwjgl-3.3.4-natives-windows.jar"},
                    "natives-linux": {"sha1": "7777777777777777777777777777777777777777", "size": 67891, "url": "https://libraries.minecraft.net/org/lwjgl/lwjgl/3.3.4/lwjgl-3.3.4-natives-linux.jar"}
                }},
             "rules": [{"action": "allow", "os": {"name": "linux"}}],
             "natives": {"windows": "natives-windows", "linux": "natives-linux"},
             "extract": {"exclude": ["META-INF/"]}}],
        "logging": {
            "client": {
                "argument": "-Dlog4j.configurationFile=${path}",
                "file": {"id": "client-1.21.xml", "sha1": "8888888888888888888888888888888888888888", "size": 2068, "url": "https://piston-meta.mojang.com/v1/packages/8888888888888888888888888888888888888888/client-1.21.xml"},
                "type": "log4j2-xml"
            }
        },
        "minimumLauncherVersion": 21,
        "time": "2024-12-03T12:35:58+00:00",
        "releaseTime": "2024-12-03T09:23:39+00:00"
    }"#;

    const LEGACY_JSON: &str = r#"{
        "id": "1.8.9",
        "type": "release",
        "mainClass": "net.minecraft.client.main.Main",
        "minecraftArguments": "--username ${auth_player_name} --version ${version_name}",
        "assetIndex": {
            "id": "1.8",
            "sha1": "9999999999999999999999999999999999999999",
            "size": 345678,
            "totalSize": 1747368786,
            "url": "https://s3.amazonaws.com/Minecraft.Download/indexes/1.8.json"
        },
        "downloads": {
            "client": {"sha1": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "size": 7888445, "url": "https://piston-data.mojang.com/v1/objects/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/client.jar"}
        },
        "libraries": [
            {"name": "net.java.jinput:jinput:2.0.5",
             "downloads": {"artifact": {"sha1": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "size": 208785, "url": "https://libraries.minecraft.net/net/java/jinput/jinput/2.0.5/jinput-2.0.5.jar"}}}
        ],
        "minimumLauncherVersion": 7,
        "time": "2015-12-07T17:27:30+00:00",
        "releaseTime": "2015-12-07T17:27:30+00:00"
    }"#;

    #[test]
    fn parses_modern_version_json() {
        let v: VersionJson = parse(MODERN_JSON.as_bytes()).expect("parse");

        assert_eq!(v.id, "1.21.4");
        assert_eq!(v.kind, "release");
        assert_eq!(
            v.main_class.as_deref(),
            Some("net.minecraft.client.main.Main")
        );
        assert_eq!(v.minimum_launcher_version, Some(21));

        let args = v.arguments.expect("arguments");
        assert_eq!(args.game.len(), 3);
        assert!(matches!(args.game[0], Argument::Plain(_)));
        let Argument::Ruled(ruled) = &args.game[2] else {
            panic!("expected ruled game argument");
        };
        assert_eq!(ruled.rules[0].action, "allow");
        assert!(ruled.rules[0].features.as_ref().expect("features")["has_custom_resolution"]);
        let ArgumentValue::Multi(values) = &ruled.value else {
            panic!("expected multi-value argument");
        };
        assert_eq!(values[0], "--width");
        assert_eq!(args.jvm.len(), 2);

        assert_eq!(v.asset_index.as_ref().expect("asset index").id, "25");
        assert_eq!(v.assets.as_deref(), Some("25"));
        assert_eq!(
            v.java_version.as_ref().expect("java version").major_version,
            21
        );

        let client = v.downloads.client.expect("client download");
        assert!(client.url.ends_with("client.jar"));
        assert!(v.downloads.server.is_some());

        assert_eq!(v.libraries.len(), 1);
        let lib = &v.libraries[0];
        assert_eq!(lib.name, "org.lwjgl:lwjgl:3.3.4");
        assert_eq!(
            lib.extract.as_ref().expect("extract").exclude,
            ["META-INF/"]
        );
        let natives = lib.natives.as_ref().expect("natives");
        assert_eq!(natives["windows"], "natives-windows");
        let classifiers = lib
            .downloads
            .as_ref()
            .expect("downloads")
            .classifiers
            .as_ref()
            .expect("classifiers");
        assert!(classifiers.contains_key("natives-linux"));
        assert!(classifiers.contains_key("natives-windows"));

        let logging = v.logging.expect("logging");
        assert_eq!(logging.client.argument, "-Dlog4j.configurationFile=${path}");
        assert_eq!(logging.client.kind, "log4j2-xml");
    }

    #[test]
    fn parses_legacy_version_json() {
        let v: VersionJson = parse(LEGACY_JSON.as_bytes()).expect("parse");

        assert_eq!(v.id, "1.8.9");
        assert!(v.arguments.is_none());
        assert!(v.minecraft_arguments.is_some());
        assert!(v.java_version.is_none());
        assert!(v.logging.is_none());
        assert_eq!(v.minimum_launcher_version, Some(7));
        assert_eq!(v.libraries.len(), 1);
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(parse(b"not json").is_err());
        assert!(parse(b"{}").is_err());
    }
}
