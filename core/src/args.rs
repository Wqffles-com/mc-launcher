//! JVM and game argument resolution.
//!
//! Modern versions (1.13+) ship a structured `arguments` block; older ones
//! carry a single `minecraftArguments` template string. Both use `${token}`
//! placeholders that this module expands from a template populated by the
//! launch engine.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::Result;
use crate::rules::{Features, Platform, resolve_arguments};
use crate::version_json::VersionJson;

/// Token names used by Minecraft argument templates.
pub const TOKEN_AUTH_PLAYER_NAME: &str = "auth_player_name";
pub const TOKEN_VERSION_NAME: &str = "version_name";
pub const TOKEN_VERSION_TYPE: &str = "version_type";
pub const TOKEN_GAME_DIRECTORY: &str = "game_directory";
pub const TOKEN_GAME_ASSETS: &str = "game_assets";
pub const TOKEN_ASSETS_ROOT: &str = "assets_root";
pub const TOKEN_ASSETS_INDEX_NAME: &str = "assets_index_name";
pub const TOKEN_AUTH_UUID: &str = "auth_uuid";
pub const TOKEN_AUTH_ACCESS_TOKEN: &str = "auth_access_token";
pub const TOKEN_USER_TYPE: &str = "user_type";
pub const TOKEN_USER_PROPERTIES: &str = "user_properties";
pub const TOKEN_NATIVES_DIRECTORY: &str = "natives_directory";
pub const TOKEN_LAUNCHER_NAME: &str = "launcher_name";
pub const TOKEN_LAUNCHER_VERSION: &str = "launcher_version";
pub const TOKEN_CLASSPATH: &str = "classpath";
pub const TOKEN_CLASSPATH_SEPARATOR: &str = "classpath_separator";
pub const TOKEN_RESOLUTION_WIDTH: &str = "resolution_width";
pub const TOKEN_RESOLUTION_HEIGHT: &str = "resolution_height";

/// A resolved set of `{token -> value}` substitutions.
#[derive(Debug, Clone, Default)]
pub struct Template {
    map: BTreeMap<String, String>,
}

impl Template {
    /// Insert a token value.
    pub fn insert(&mut self, name: &str, value: impl Into<String>) {
        self.map.insert(name.to_owned(), value.into());
    }

    /// Expand every `${name}` placeholder; unknown tokens expand to empty.
    #[must_use]
    pub fn expand(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find("${") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find('}') else {
                out.push_str(&rest[start..]);
                return out;
            };
            let name = &after[..end];
            if let Some(value) = self.map.get(name) {
                out.push_str(value);
            }
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        out
    }

    /// Expand every argument in `args`.
    #[must_use]
    pub fn expand_args(&self, args: &[String]) -> Vec<String> {
        args.iter().map(|arg| self.expand(arg)).collect()
    }
}

/// Resolve the final game arguments for a version: rules-filtered arguments
/// from the modern block, or the legacy template split on whitespace.
///
/// # Errors
///
/// Never fails; present to allow future validation.
pub fn game_arguments(
    version: &VersionJson,
    template: &Template,
    platform: &Platform,
    features: &Features,
) -> Result<Vec<String>> {
    let resolved = match &version.arguments {
        Some(args) => resolve_arguments(&args.game, platform, features),
        None => version
            .minecraft_arguments
            .as_deref()
            .map_or_else(Vec::new, split_legacy),
    };
    Ok(template.expand_args(&resolved))
}

/// Resolve the JVM arguments: rules-filtered modern block, or the default
/// set for legacy versions (library path + classpath).
///
/// # Errors
///
/// Never fails; present to allow future validation.
pub fn jvm_arguments(
    version: &VersionJson,
    template: &Template,
    platform: &Platform,
    features: &Features,
) -> Result<Vec<String>> {
    let resolved = match &version.arguments {
        Some(args) => resolve_arguments(&args.jvm, platform, features),
        None => vec![
            "-Djava.library.path=${natives_directory}".to_owned(),
            "-cp ${classpath}".to_owned(),
        ],
    };
    Ok(template.expand_args(&resolved))
}

/// Split a legacy `minecraftArguments` template into arguments. MC's legacy
/// templates never contain quoted values, so whitespace splitting suffices.
fn split_legacy(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

/// Build a classpath string from paths, joined with the platform separator.
#[must_use]
pub fn classpath(paths: &[&Path]) -> String {
    let sep = if cfg!(target_os = "windows") {
        ";"
    } else {
        ":"
    };
    paths
        .iter()
        .map(|p| p.to_string_lossy())
        .collect::<Vec<_>>()
        .join(sep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Platform;
    use crate::version_json::{Argument, ArgumentValue, Arguments, Rule, RuledArgument};

    #[test]
    fn expands_known_and_unknown_tokens() {
        let mut template = Template::default();
        template.insert("auth_player_name", "Steve");
        assert_eq!(
            template.expand("--username ${auth_player_name} --x ${missing} --y"),
            "--username Steve --x  --y"
        );
    }

    #[test]
    fn expands_without_tokens_verbatim() {
        let template = Template::default();
        assert_eq!(template.expand("--plain"), "--plain");
    }

    #[test]
    fn resolves_modern_game_arguments_with_rules() {
        let version = VersionJson {
            id: "1.21.4".to_owned(),
            kind: "release".to_owned(),
            main_class: Some("net.minecraft.client.main.Main".to_owned()),
            arguments: Some(Arguments {
                game: vec![
                    Argument::Plain("--username".to_owned()),
                    Argument::Plain("${auth_player_name}".to_owned()),
                    Argument::Ruled(RuledArgument {
                        rules: vec![Rule {
                            action: "allow".to_owned(),
                            os: None,
                            features: Some(BTreeMap::from([(
                                "has_custom_resolution".to_owned(),
                                true,
                            )])),
                        }],
                        value: ArgumentValue::Multi(vec![
                            "--width".to_owned(),
                            "${resolution_width}".to_owned(),
                        ]),
                    }),
                ],
                jvm: Vec::new(),
            }),
            minecraft_arguments: None,
            asset_index: None,
            assets: None,
            java_version: None,
            downloads: crate::version_json::Downloads::default(),
            libraries: Vec::new(),
            logging: None,
            minimum_launcher_version: None,
            time: String::new(),
            release_time: String::new(),
        };
        let platform = Platform::current();
        let mut template = Template::default();
        template.insert("auth_player_name", "Alex");
        template.insert("resolution_width", "1920");

        let without =
            game_arguments(&version, &template, &platform, &Features::default()).expect("args");
        assert_eq!(without, vec!["--username", "Alex"]);

        let features = Features {
            has_custom_resolution: true,
            ..Features::default()
        };
        let with = game_arguments(&version, &template, &platform, &features).expect("args");
        assert_eq!(with, vec!["--username", "Alex", "--width", "1920"]);
    }

    #[test]
    fn resolves_legacy_arguments() {
        let version = VersionJson {
            id: "1.8.9".to_owned(),
            kind: "release".to_owned(),
            main_class: Some("net.minecraft.client.main.Main".to_owned()),
            arguments: None,
            minecraft_arguments: Some(
                "--username ${auth_player_name} --gameDir ${game_directory}".to_owned(),
            ),
            asset_index: None,
            assets: None,
            java_version: None,
            downloads: crate::version_json::Downloads::default(),
            libraries: Vec::new(),
            logging: None,
            minimum_launcher_version: None,
            time: String::new(),
            release_time: String::new(),
        };
        let mut template = Template::default();
        template.insert("auth_player_name", "Steve");
        template.insert("game_directory", "D:/games");
        let args = game_arguments(
            &version,
            &template,
            &Platform::current(),
            &Features::default(),
        )
        .expect("args");
        assert_eq!(args, vec!["--username", "Steve", "--gameDir", "D:/games"]);
    }

    #[test]
    fn legacy_versions_get_default_jvm_args() {
        let version = VersionJson {
            id: "1.8.9".to_owned(),
            kind: "release".to_owned(),
            main_class: None,
            arguments: None,
            minecraft_arguments: None,
            asset_index: None,
            assets: None,
            java_version: None,
            downloads: crate::version_json::Downloads::default(),
            libraries: Vec::new(),
            logging: None,
            minimum_launcher_version: None,
            time: String::new(),
            release_time: String::new(),
        };
        let mut template = Template::default();
        template.insert("natives_directory", "D:/natives");
        template.insert("classpath", "D:/cp");
        let args = jvm_arguments(
            &version,
            &template,
            &Platform::current(),
            &Features::default(),
        )
        .expect("args");
        assert_eq!(args, vec!["-Djava.library.path=D:/natives", "-cp D:/cp"]);
    }

    #[test]
    fn classpath_uses_platform_separator() {
        let joined = classpath(&[Path::new("C:/a.jar"), Path::new("C:/b.jar")]);
        if cfg!(target_os = "windows") {
            assert_eq!(joined, "C:/a.jar;C:/b.jar");
        } else {
            assert_eq!(joined, "C:/a.jar:C:/b.jar");
        }
    }
}
