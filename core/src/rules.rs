//! Platform rule evaluation and library selection.
//!
//! Version JSONs gate libraries, natives and arguments on rules over the
//! current OS/arch and launcher features (`has_custom_resolution`, ...).
//! This module implements Mojang's rule semantics: a rules list denies by
//! default, is evaluated in order, and the last matching rule wins.

use std::path::PathBuf;

use crate::error::{Error, Result};
use crate::version_json::{Argument, ArgumentValue, ArtifactDownload, Library, Rule, VersionJson};

/// Fallback host for libraries whose version JSON does not carry a download
/// URL (common in older versions).
pub const MAVEN_FALLBACK_HOST: &str = "https://libraries.minecraft.net";

/// The platform a launch is being prepared for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platform {
    /// `windows`, `linux` or `osx` (Mojang's naming).
    pub os: &'static str,
    /// `x86`, `x86_64`, `arm` or `arm64` (Mojang's naming).
    pub arch: &'static str,
}

impl Platform {
    /// The platform the launcher is running on.
    #[must_use]
    pub fn current() -> Self {
        Self {
            os: if cfg!(target_os = "windows") {
                "windows"
            } else if cfg!(target_os = "macos") {
                "osx"
            } else {
                "linux"
            },
            arch: match std::env::consts::ARCH {
                "x86" => "x86",
                "x86_64" => "x86_64",
                "aarch64" => "arm64",
                "arm" => "arm",
                other => other,
            },
        }
    }
}

/// Launcher feature flags referenced by rules.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct Features {
    pub has_custom_resolution: bool,
    pub is_demo_user: bool,
    pub has_quick_plays_support: bool,
    pub is_quick_play_singleplayer: bool,
    pub is_quick_play_multiplayer: bool,
    pub is_quick_play_realms: bool,
}

impl Features {
    /// Look up a feature flag by its rule name; unknown flags are `false`.
    #[must_use]
    pub fn get(&self, name: &str) -> bool {
        match name {
            "has_custom_resolution" => self.has_custom_resolution,
            "is_demo_user" => self.is_demo_user,
            "has_quick_plays_support" => self.has_quick_plays_support,
            "is_quick_play_singleplayer" => self.is_quick_play_singleplayer,
            "is_quick_play_multiplayer" => self.is_quick_play_multiplayer,
            "is_quick_play_realms" => self.is_quick_play_realms,
            _ => false,
        }
    }
}

fn rule_matches(rule: &Rule, platform: &Platform, features: &Features) -> bool {
    if let Some(os) = &rule.os {
        if let Some(name) = &os.name
            && name != platform.os
        {
            return false;
        }
        if let Some(arch) = &os.arch
            && arch != platform.arch
        {
            return false;
        }
        // `os.version` regexes are not supported; treat such rules as
        // non-matching.
        if os.version.is_some() {
            return false;
        }
    }
    if let Some(feats) = &rule.features {
        for (name, required) in feats {
            if features.get(name) != *required {
                return false;
            }
        }
    }
    true
}

/// Whether a rules list permits the item. Rules deny by default; the last
/// matching rule wins.
#[must_use]
pub fn allowed(rules: &[Rule], platform: &Platform, features: &Features) -> bool {
    let mut result = false;
    for rule in rules {
        if rule_matches(rule, platform, features) {
            result = rule.action == "allow";
        }
    }
    result
}

/// Whether a library is included on this platform.
#[must_use]
pub fn library_allowed(lib: &Library, platform: &Platform, features: &Features) -> bool {
    lib.rules
        .as_deref()
        .is_none_or(|rules| allowed(rules, platform, features))
}

/// A concrete file a library resolves to on a platform.
#[derive(Debug, Clone)]
pub struct LibraryFile {
    /// Storage path under the downloads root, in maven layout.
    pub path: PathBuf,
    /// The artifact to fetch. Empty `sha1`/`size` mean "no verification".
    pub download: ArtifactDownload,
    /// Whether the archive must be unpacked into the natives directory.
    pub extract: bool,
    /// Archive entry prefixes to skip when extracting.
    pub exclude: Vec<String>,
}

/// Resolve the file (plain artifact or platform native classifier) a library
/// provides on this platform. `None` means the library has no artifact for
/// this platform (e.g. natives for another OS).
///
/// Libraries without an explicit download URL fall back to the maven host.
///
/// # Errors
///
/// Fails on malformed maven coordinates.
pub fn library_file(lib: &Library, platform: &Platform) -> Result<Option<LibraryFile>> {
    let downloads = lib.downloads.as_ref();
    let (classifier, extract, download) = if let Some(natives) = &lib.natives {
        let Some(base) = natives.get(platform.os) else {
            return Ok(None);
        };
        // Old versions embed a literal `${arch}` in the classifier, resolved
        // to `64` (or `32` on 32-bit x86).
        let base = base.replace("${arch}", if platform.arch == "x86" { "32" } else { "64" });
        let classifiers = downloads.and_then(|d| d.classifiers.as_ref());
        let key = if platform.arch == "arm64"
            && classifiers.is_some_and(|c| c.contains_key(&format!("{base}-arm64")))
        {
            format!("{base}-arm64")
        } else {
            base.clone()
        };
        let download = classifiers.and_then(|c| c.get(&key).cloned());
        (Some(key), true, download)
    } else {
        (None, false, downloads.and_then(|d| d.artifact.clone()))
    };

    let path = maven_path(&lib.name, classifier.as_deref())?;
    let url_path = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    let download = download.unwrap_or_else(|| ArtifactDownload {
        sha1: String::new(),
        size: 0,
        url: format!("{MAVEN_FALLBACK_HOST}/{url_path}"),
    });
    let exclude = lib
        .extract
        .as_ref()
        .map_or_else(Vec::new, |extract| extract.exclude.clone());
    Ok(Some(LibraryFile {
        path,
        download,
        extract,
        exclude,
    }))
}

/// Resolve every library file a version needs on this platform, preserving
/// version JSON order (classpath order matters). Libraries with both a plain
/// artifact and platform natives contribute both files, mirroring the
/// official launcher's classpath.
///
/// # Errors
///
/// Fails on malformed maven coordinates.
pub fn resolve_libraries(
    version: &VersionJson,
    platform: &Platform,
    features: &Features,
) -> Result<Vec<LibraryFile>> {
    let mut out = Vec::new();
    for library in &version.libraries {
        if !library_allowed(library, platform, features) {
            continue;
        }
        if let Some(file) = library_file(library, platform)? {
            if library.natives.is_some()
                && let Some(artifact) = library.downloads.as_ref().and_then(|d| d.artifact.as_ref())
            {
                out.push(LibraryFile {
                    path: maven_path(&library.name, None)?,
                    download: artifact.clone(),
                    extract: false,
                    exclude: Vec::new(),
                });
            }
            out.push(file);
        }
    }
    Ok(out)
}

/// Expand an argument list, dropping ruled arguments whose rules deny them.
#[must_use]
pub fn resolve_arguments(
    args: &[Argument],
    platform: &Platform,
    features: &Features,
) -> Vec<String> {
    args.iter()
        .flat_map(|arg| match arg {
            Argument::Plain(value) => vec![value.clone()],
            Argument::Ruled(ruled) => {
                if allowed(&ruled.rules, platform, features) {
                    match &ruled.value {
                        ArgumentValue::Single(value) => vec![value.clone()],
                        ArgumentValue::Multi(values) => values.clone(),
                    }
                } else {
                    Vec::new()
                }
            }
        })
        .collect()
}

/// Filesystem path for a maven coordinate: `org.lwjgl:lwjgl:3.3.4` becomes
/// `org/lwjgl/lwjgl/3.3.4/lwjgl-3.3.4.jar`. An optional classifier inserts
/// `-<classifier>` before the extension; modern manifests may embed it as a
/// fourth coordinate part (`com.mojang:jtracy:1.0.37:natives-windows`), and a
/// rare `@ext` suffix changes the extension. An explicit `classifier`
/// argument wins over one embedded in the name.
///
/// # Errors
///
/// Fails if the coordinate does not have group:artifact:version
/// (plus an optional classifier part).
pub fn maven_path(name: &str, classifier: Option<&str>) -> Result<PathBuf> {
    let (coords, extension) = name.split_once('@').unwrap_or((name, "jar"));
    let mut parts = coords.split(':');
    let group = parts
        .next()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| Error::InvalidMavenName(name.to_owned()))?;
    let artifact = parts
        .next()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| Error::InvalidMavenName(name.to_owned()))?;
    let version = parts
        .next()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| Error::InvalidMavenName(name.to_owned()))?;
    let embedded = parts.next().filter(|p| !p.is_empty()).map(str::to_owned);
    if parts.next().is_some() {
        return Err(Error::InvalidMavenName(name.to_owned()));
    }
    let classifier = classifier.map(str::to_owned).or(embedded);
    let mut file_name = format!("{artifact}-{version}");
    if let Some(classifier) = classifier {
        file_name.push('-');
        file_name.push_str(&classifier);
    }
    file_name.push('.');
    file_name.push_str(extension);

    let mut path = PathBuf::new();
    for part in group.split('.') {
        path.push(part);
    }
    path.push(artifact);
    path.push(version);
    path.push(file_name);
    Ok(path)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::version_json::{Extract, Library, OsRule};

    fn rule(action: &str, os_name: Option<&str>) -> Rule {
        Rule {
            action: action.to_owned(),
            os: os_name.map(|name| OsRule {
                name: Some(name.to_owned()),
                arch: None,
                version: None,
            }),
            features: None,
        }
    }

    #[test]
    fn empty_rules_deny() {
        let platform = Platform::current();
        assert!(!allowed(&[], &platform, &Features::default()));
    }

    #[test]
    fn os_rules_gate_on_name() {
        let platform = Platform {
            os: "windows",
            arch: "x86_64",
        };
        assert!(allowed(
            &[rule("allow", Some("windows"))],
            &platform,
            &Features::default()
        ));
        let linux = Platform {
            os: "linux",
            arch: "x86_64",
        };
        assert!(!allowed(
            &[rule("allow", Some("windows"))],
            &linux,
            &Features::default()
        ));
    }

    #[test]
    fn last_matching_rule_wins() {
        let platform = Platform {
            os: "osx",
            arch: "arm64",
        };
        let rules = [rule("allow", Some("osx")), rule("disallow", Some("osx"))];
        assert!(!allowed(&rules, &platform, &Features::default()));
        let rules = [rule("disallow", Some("osx")), rule("allow", Some("osx"))];
        assert!(allowed(&rules, &platform, &Features::default()));
    }

    #[test]
    fn rules_can_reference_features() {
        let platform = Platform::current();
        let rules = [Rule {
            action: "allow".to_owned(),
            os: None,
            features: Some(BTreeMap::from([("has_custom_resolution".to_owned(), true)])),
        }];
        assert!(!allowed(&rules, &platform, &Features::default()));
        let features = Features {
            has_custom_resolution: true,
            ..Features::default()
        };
        assert!(allowed(&rules, &platform, &features));
    }

    #[test]
    fn os_version_rules_do_not_match() {
        let rule = Rule {
            action: "allow".to_owned(),
            os: Some(OsRule {
                name: None,
                arch: None,
                version: Some("^10\\.".to_owned()),
            }),
            features: None,
        };
        assert!(!allowed(
            &[rule],
            &Platform::current(),
            &Features::default()
        ));
    }

    #[test]
    fn library_without_rules_is_allowed() {
        let lib = Library {
            name: "org.example:lib:1.0".to_owned(),
            downloads: None,
            rules: None,
            natives: None,
            extract: None,
        };
        assert!(library_allowed(
            &lib,
            &Platform::current(),
            &Features::default()
        ));
    }

    #[test]
    fn disallowed_library_is_excluded() {
        let windows = Platform {
            os: "windows",
            arch: "x86_64",
        };
        let linux = Platform {
            os: "linux",
            arch: "x86_64",
        };
        // A lone disallow rule denies everywhere (rules deny by default)...
        let lib = Library {
            name: "org.example:lib:1.0".to_owned(),
            downloads: None,
            rules: Some(vec![rule("disallow", Some("windows"))]),
            natives: None,
            extract: None,
        };
        assert!(!library_allowed(&lib, &windows, &Features::default()));
        assert!(!library_allowed(&lib, &linux, &Features::default()));
        // ...but an explicit allow for other platforms overrides it.
        let lib = Library {
            name: "org.example:lib:1.0".to_owned(),
            downloads: None,
            rules: Some(vec![
                rule("disallow", Some("windows")),
                rule("allow", Some("linux")),
            ]),
            natives: None,
            extract: None,
        };
        assert!(!library_allowed(&lib, &windows, &Features::default()));
        assert!(library_allowed(&lib, &linux, &Features::default()));
    }

    #[test]
    fn selects_plain_artifact_by_default() {
        let lib = Library {
            name: "org.example:lib:1.0".to_owned(),
            downloads: None,
            rules: None,
            natives: None,
            extract: None,
        };
        let file = library_file(&lib, &Platform::current())
            .expect("file")
            .expect("some");
        assert_eq!(file.path, PathBuf::from("org/example/lib/1.0/lib-1.0.jar"));
        assert!(
            file.download
                .url
                .ends_with("org/example/lib/1.0/lib-1.0.jar")
        );
        assert!(!file.extract);
        assert!(file.download.sha1.is_empty());
    }

    #[test]
    fn uses_download_when_present() {
        let lib = Library {
            name: "org.example:lib:1.0".to_owned(),
            downloads: Some(crate::version_json::LibraryDownloads {
                artifact: Some(ArtifactDownload {
                    sha1: "abc".to_owned(),
                    size: 42,
                    url: "https://cdn.example/lib.jar".to_owned(),
                }),
                classifiers: None,
            }),
            rules: None,
            natives: None,
            extract: None,
        };
        let file = library_file(&lib, &Platform::current())
            .expect("file")
            .expect("some");
        assert_eq!(file.download.url, "https://cdn.example/lib.jar");
        assert_eq!(file.download.sha1, "abc");
    }

    #[test]
    fn picks_native_classifier_per_os() {
        let natives = BTreeMap::from([
            ("windows".to_owned(), "natives-windows".to_owned()),
            ("linux".to_owned(), "natives-linux".to_owned()),
        ]);
        let lib = Library {
            name: "org.lwjgl:lwjgl:3.3.4".to_owned(),
            downloads: Some(crate::version_json::LibraryDownloads {
                artifact: Some(ArtifactDownload {
                    sha1: "a".to_owned(),
                    size: 1,
                    url: "u".to_owned(),
                }),
                classifiers: Some(BTreeMap::from([
                    (
                        "natives-windows".to_owned(),
                        ArtifactDownload {
                            sha1: "b".to_owned(),
                            size: 2,
                            url: "w".to_owned(),
                        },
                    ),
                    (
                        "natives-linux".to_owned(),
                        ArtifactDownload {
                            sha1: "c".to_owned(),
                            size: 3,
                            url: "l".to_owned(),
                        },
                    ),
                ])),
            }),
            rules: None,
            natives: Some(natives),
            extract: Some(Extract {
                exclude: vec!["META-INF/".to_owned()],
            }),
        };
        let windows = Platform {
            os: "windows",
            arch: "x86_64",
        };
        let file = library_file(&lib, &windows).expect("file").expect("some");
        assert_eq!(
            file.path,
            PathBuf::from("org/lwjgl/lwjgl/3.3.4/lwjgl-3.3.4-natives-windows.jar")
        );
        assert_eq!(file.download.url, "w");
        assert!(file.extract);

        let linux = Platform {
            os: "linux",
            arch: "x86_64",
        };
        let file = library_file(&lib, &linux).expect("file").expect("some");
        assert_eq!(
            file.path,
            PathBuf::from("org/lwjgl/lwjgl/3.3.4/lwjgl-3.3.4-natives-linux.jar")
        );
    }

    #[test]
    fn arm64_prefers_arm64_native_classifier() {
        let lib = Library {
            name: "org.lwjgl:lwjgl:3.3.4".to_owned(),
            downloads: Some(crate::version_json::LibraryDownloads {
                artifact: None,
                classifiers: Some(BTreeMap::from([
                    (
                        "natives-osx".to_owned(),
                        ArtifactDownload {
                            sha1: "x".to_owned(),
                            size: 1,
                            url: "osx".to_owned(),
                        },
                    ),
                    (
                        "natives-osx-arm64".to_owned(),
                        ArtifactDownload {
                            sha1: "y".to_owned(),
                            size: 2,
                            url: "osx-arm64".to_owned(),
                        },
                    ),
                ])),
            }),
            rules: None,
            natives: Some(BTreeMap::from([(
                "osx".to_owned(),
                "natives-osx".to_owned(),
            )])),
            extract: None,
        };
        let arm = Platform {
            os: "osx",
            arch: "arm64",
        };
        let file = library_file(&lib, &arm).expect("file").expect("some");
        assert_eq!(file.download.url, "osx-arm64");
        let intel = Platform {
            os: "osx",
            arch: "x86_64",
        };
        let file = library_file(&lib, &intel).expect("file").expect("some");
        assert_eq!(file.download.url, "osx");
    }

    #[test]
    fn no_native_for_os_means_none() {
        let lib = Library {
            name: "org.example:lib:1.0".to_owned(),
            downloads: None,
            rules: None,
            natives: Some(BTreeMap::from([(
                "windows".to_owned(),
                "natives-windows".to_owned(),
            )])),
            extract: None,
        };
        let linux = Platform {
            os: "linux",
            arch: "x86_64",
        };
        assert!(library_file(&lib, &linux).expect("file").is_none());
    }

    #[test]
    fn natives_resolve_legacy_arch_placeholder() {
        let lib = Library {
            name: "tv.twitch:twitch-platform:6.5".to_owned(),
            downloads: Some(crate::version_json::LibraryDownloads {
                artifact: None,
                classifiers: Some(BTreeMap::from([(
                    "natives-windows-64".to_owned(),
                    ArtifactDownload {
                        sha1: "z".to_owned(),
                        size: 9,
                        url: "twitch-64".to_owned(),
                    },
                )])),
            }),
            rules: None,
            natives: Some(BTreeMap::from([(
                "windows".to_owned(),
                "natives-windows-${arch}".to_owned(),
            )])),
            extract: None,
        };
        let x64 = Platform {
            os: "windows",
            arch: "x86_64",
        };
        let file = library_file(&lib, &x64).expect("file").expect("some");
        assert_eq!(file.download.url, "twitch-64");
        assert_eq!(
            file.path,
            PathBuf::from(
                "tv/twitch/twitch-platform/6.5/twitch-platform-6.5-natives-windows-64.jar"
            )
        );
        let x86 = Platform {
            os: "windows",
            arch: "x86",
        };
        let file = library_file(&lib, &x86).expect("file").expect("some");
        assert_eq!(
            file.path,
            PathBuf::from(
                "tv/twitch/twitch-platform/6.5/twitch-platform-6.5-natives-windows-32.jar"
            )
        );
    }

    #[test]
    fn maven_path_shapes() {
        assert_eq!(
            maven_path("org.lwjgl:lwjgl:3.3.4", None).expect("path"),
            PathBuf::from("org/lwjgl/lwjgl/3.3.4/lwjgl-3.3.4.jar")
        );
        assert_eq!(
            maven_path(
                "net.java.jinput:jinput-platform:2.0.5",
                Some("natives-windows")
            )
            .expect("path"),
            PathBuf::from(
                "net/java/jinput/jinput-platform/2.0.5/jinput-platform-2.0.5-natives-windows.jar"
            )
        );
        assert_eq!(
            maven_path("net.minecraft:client:1.21.4@slim", None).expect("path"),
            PathBuf::from("net/minecraft/client/1.21.4/client-1.21.4.slim")
        );
        assert_eq!(
            maven_path("com.mojang:jtracy:1.0.37:natives-windows", None).expect("path"),
            PathBuf::from("com/mojang/jtracy/1.0.37/jtracy-1.0.37-natives-windows.jar")
        );
        assert_eq!(
            maven_path(
                "com.mojang:jtracy:1.0.37:natives-windows",
                Some("natives-windows")
            )
            .expect("path"),
            PathBuf::from("com/mojang/jtracy/1.0.37/jtracy-1.0.37-natives-windows.jar")
        );
    }

    #[test]
    fn maven_path_rejects_malformed_coordinates() {
        for bad in ["no-colons", ":a:b", "a::b", "a:b", "a:b:c:d:e"] {
            assert!(
                matches!(maven_path(bad, None), Err(Error::InvalidMavenName(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn resolve_libraries_keeps_order_and_filters() {
        let libs = vec![
            Library {
                name: "org.example:first:1.0".to_owned(),
                downloads: None,
                rules: None,
                natives: None,
                extract: None,
            },
            Library {
                name: "org.example:skip:1.0".to_owned(),
                downloads: None,
                rules: Some(vec![rule("disallow", Some("windows"))]),
                natives: None,
                extract: None,
            },
            Library {
                name: "org.example:third:1.0".to_owned(),
                downloads: None,
                rules: None,
                natives: None,
                extract: None,
            },
        ];
        let version = crate::version_json::VersionJson {
            id: "t".to_owned(),
            kind: "release".to_owned(),
            main_class: None,
            arguments: None,
            minecraft_arguments: None,
            asset_index: None,
            assets: None,
            java_version: None,
            downloads: crate::version_json::Downloads::default(),
            libraries: libs,
            logging: None,
            minimum_launcher_version: None,
            time: String::new(),
            release_time: String::new(),
        };
        let windows = Platform {
            os: "windows",
            arch: "x86_64",
        };
        let files = resolve_libraries(&version, &windows, &Features::default()).expect("resolve");
        let names: Vec<_> = files
            .iter()
            .map(|f| f.path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2);
        assert!(names[0].ends_with("first-1.0.jar"));
        assert!(names[1].ends_with("third-1.0.jar"));
    }

    #[test]
    fn resolve_libraries_includes_artifact_beside_natives() {
        let libs = vec![Library {
            name: "org.lwjgl.lwjgl:lwjgl-platform:2.9.4".to_owned(),
            downloads: Some(crate::version_json::LibraryDownloads {
                artifact: Some(ArtifactDownload {
                    sha1: "a".to_owned(),
                    size: 22,
                    url: "https://libraries.minecraft.net/stub.jar".to_owned(),
                }),
                classifiers: Some(BTreeMap::from([(
                    "natives-windows".to_owned(),
                    ArtifactDownload {
                        sha1: "b".to_owned(),
                        size: 2,
                        url: "native.jar".to_owned(),
                    },
                )])),
            }),
            rules: None,
            natives: Some(BTreeMap::from([(
                "windows".to_owned(),
                "natives-windows".to_owned(),
            )])),
            extract: None,
        }];
        let version = crate::version_json::VersionJson {
            id: "t".to_owned(),
            kind: "release".to_owned(),
            main_class: None,
            arguments: None,
            minecraft_arguments: None,
            asset_index: None,
            assets: None,
            java_version: None,
            downloads: crate::version_json::Downloads::default(),
            libraries: libs,
            logging: None,
            minimum_launcher_version: None,
            time: String::new(),
            release_time: String::new(),
        };
        let windows = Platform {
            os: "windows",
            arch: "x86_64",
        };
        let files = resolve_libraries(&version, &windows, &Features::default()).expect("resolve");
        assert_eq!(files.len(), 2);
        // Plain artifact first, then the platform native classifier.
        assert_eq!(
            files[0].path,
            PathBuf::from("org/lwjgl/lwjgl/lwjgl-platform/2.9.4/lwjgl-platform-2.9.4.jar")
        );
        assert!(!files[0].extract);
        assert_eq!(
            files[0].download.url,
            "https://libraries.minecraft.net/stub.jar"
        );
        assert_eq!(
            files[1].path,
            PathBuf::from(
                "org/lwjgl/lwjgl/lwjgl-platform/2.9.4/lwjgl-platform-2.9.4-natives-windows.jar"
            )
        );
        assert!(files[1].extract);
    }
}
