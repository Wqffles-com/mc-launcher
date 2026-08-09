use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use mc_launcher_core::dirs::Directories;
use mc_launcher_core::instances::{Instance, InstanceManager, Loader, LoaderKind};
use mc_launcher_core::version_json;
use mc_launcher_core::version_manifest::MANIFEST_URL;
use mc_launcher_core::version_manifest::{self, VersionManifest};

#[derive(Debug, Parser)]
#[command(
    name = "mc-launcher",
    version,
    about = "A cross-platform Minecraft launcher: install, launch and manage Minecraft versions"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize the launcher data directory (`AppData` on Windows,
    /// `~/.local/share` on Linux)
    Init,
    /// Install a Minecraft version (downloads client, libraries and assets)
    Install {
        /// Version id (e.g. 1.21.4); defaults to the latest release
        version: Option<String>,
    },
    /// Launch a Minecraft version
    Launch {
        /// Version id (e.g. 1.21.4); defaults to the latest release
        version: Option<String>,
    },
    /// Sign in with a Microsoft account (device code flow)
    Login,
    /// Manage game instances
    Instance {
        #[command(subcommand)]
        command: Option<InstanceCommand>,
    },
    /// Version manifest & metadata
    Version(VersionArgs),
}

#[derive(Debug, Subcommand)]
enum InstanceCommand {
    /// List instances
    List,
    /// Create a new instance (defaults to the latest release version)
    Create {
        /// Instance name
        name: String,
        /// Minecraft version id (e.g. 1.21.4); defaults to the latest release
        #[arg(long)]
        version: Option<String>,
    },
    /// Delete an instance (by id or name) including its game data
    Delete {
        /// Instance id or name
        instance: String,
    },
    /// Clone an instance under a new name
    Clone {
        /// Instance id or name to clone
        source: String,
        /// Name for the clone
        name: String,
    },
    /// Set the Minecraft version of an instance
    SetVersion {
        /// Instance id or name
        instance: String,
        /// Version id (e.g. 1.21.4)
        version: String,
    },
    /// Select a mod loader for an instance
    SetLoader {
        /// Instance id or name
        instance: String,
        /// Loader kind: fabric, quilt, forge or neoforge
        kind: String,
        /// Loader version (e.g. 0.16.10)
        version: String,
    },
    /// Show instance details
    Info {
        /// Instance id or name
        instance: String,
    },
    /// Export an instance to a ZIP archive
    Export {
        /// Instance id or name
        instance: String,
        /// Output archive path (defaults to <data dir>/exports/<name>-<id>.zip)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Import an instance from a ZIP archive
    Import {
        /// Archive path (.zip)
        file: PathBuf,
        /// Name for the imported instance (defaults to the archived name)
        #[arg(long)]
        name: Option<String>,
    },
}

#[derive(Debug, clap::Args)]
struct VersionArgs {
    #[command(subcommand)]
    command: VersionCommand,
}

#[derive(Debug, Subcommand)]
enum VersionCommand {
    /// List versions from the Mojang manifest
    List {
        /// Re-fetch the manifest from the network, ignoring the cache
        #[arg(long)]
        refresh: bool,
        /// Filter by type: release, snapshot, or all
        #[arg(long, default_value = "all")]
        kind: String,
    },
    /// Show details for a single version
    Info {
        /// Version id (e.g. 1.21.4)
        id: String,
        /// Re-fetch the manifest from the network, ignoring the cache
        #[arg(long)]
        refresh: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init => cmd_init(),
        Command::Install { .. } | Command::Launch { .. } => {
            anyhow::bail!(
                "not implemented yet — install/launch land with the download & launch pipeline (TASK-458cd)"
            )
        }
        Command::Login => anyhow::bail!(
            "not implemented yet — Microsoft auth lands with the device-code flow (TASK-ix345)"
        ),
        Command::Instance { command } => {
            cmd_instance(command.unwrap_or(InstanceCommand::List)).await
        }
        Command::Version(args) => cmd_version(args).await,
    }
}

fn cmd_init() -> Result<()> {
    let dirs = Directories::discover().context("could not resolve the data directory")?;
    dirs.ensure_all()?;
    println!("Initialized {}", dirs.root().display());
    Ok(())
}

async fn cmd_version(args: VersionArgs) -> Result<()> {
    let dirs = Directories::discover().context("could not resolve the data directory")?;
    match args.command {
        VersionCommand::List { refresh, kind } => {
            let manifest = load_manifest(&dirs, refresh).await?;
            let versions: Vec<_> = match kind.as_str() {
                "all" => manifest.versions.iter().collect(),
                "release" => manifest.of_kind("release").collect(),
                "snapshot" => manifest.of_kind("snapshot").collect(),
                other => anyhow::bail!(
                    "unknown type filter '{other}' (expected all, release or snapshot)"
                ),
            };
            println!("ID               TYPE       RELEASE TIME         SHA1");
            for v in &versions {
                println!(
                    "{:<16} {:<10} {:<20} {}",
                    v.id,
                    v.kind,
                    v.release_time,
                    v.sha1.as_deref().unwrap_or("-")
                );
            }
            println!(
                "\n{} versions (latest release: {}, latest snapshot: {})",
                versions.len(),
                manifest.latest.release,
                manifest.latest.snapshot
            );
        }
        VersionCommand::Info { id, refresh } => {
            let manifest = load_manifest(&dirs, refresh).await?;
            let info = manifest.find(&id).ok_or_else(|| {
                anyhow::anyhow!(
                    "version '{id}' not found in manifest — try `mc-launcher version list`"
                )
            })?;
            let version = version_json::fetch(&http_client(), &info.url)
                .await
                .context(format!("failed to fetch version JSON for '{id}'"))?;

            println!("id:                {}", version.id);
            println!("type:              {}", version.kind);
            println!("release time:      {}", version.release_time);
            println!(
                "main class:        {}",
                version.main_class.as_deref().unwrap_or("-")
            );
            println!(
                "assets:            {} ({})",
                version.asset_index.id,
                version.assets.as_deref().unwrap_or("-")
            );
            println!(
                "java:              {} (major {})",
                version
                    .java_version
                    .as_ref()
                    .map_or("-", |j| j.component.as_str()),
                version.java_version.as_ref().map_or(0, |j| j.major_version)
            );
            println!("libraries:         {}", version.libraries.len());
            println!(
                "client download:   {} bytes",
                version.downloads.client.as_ref().map_or(0, |d| d.size)
            );
            println!(
                "arguments:         {} game, {} jvm",
                version.arguments.as_ref().map_or(0, |a| a.game.len()),
                version.arguments.as_ref().map_or(0, |a| a.jvm.len())
            );
        }
    }
    Ok(())
}

async fn cmd_instance(command: InstanceCommand) -> Result<()> {
    let dirs = Directories::discover().context("could not resolve the data directory")?;
    dirs.ensure_all()?;
    let manager = InstanceManager::new(dirs);
    match command {
        InstanceCommand::List => {
            let instances = manager.list()?;
            if instances.is_empty() {
                println!("No instances yet — create one with `mc-launcher instance create <name>`");
                return Ok(());
            }
            print_instance_table(&instances);
        }
        InstanceCommand::Create { name, version } => {
            cmd_instance_create(&manager, &name, version).await?;
        }
        InstanceCommand::Delete { instance } => {
            manager.delete(&instance)?;
            println!("Deleted instance '{instance}'");
        }
        InstanceCommand::Clone { source, name } => {
            let clone = manager.clone(&source, &name)?;
            println!(
                "Cloned '{}' into '{}' ({})",
                source,
                clone.name(),
                clone.id()
            );
        }
        InstanceCommand::SetVersion { instance, version } => {
            if load_manifest(manager.dirs(), false)
                .await?
                .find(&version)
                .is_none()
            {
                anyhow::bail!(
                    "version '{version}' not found in manifest — try `mc-launcher version list`"
                );
            }
            let updated = manager.set_version(&instance, &version)?;
            println!(
                "'{}' now uses Minecraft {}",
                updated.name(),
                updated.version()
            );
        }
        InstanceCommand::SetLoader {
            instance,
            kind,
            version,
        } => {
            let kind = LoaderKind::parse(&kind).ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown loader '{kind}' (expected fabric, quilt, forge or neoforge)"
                )
            })?;
            let updated = manager.set_loader(&instance, kind, &version)?;
            println!(
                "'{}' now uses {}",
                updated.name(),
                loader_label(updated.loader())
            );
        }
        InstanceCommand::Info { instance } => print_instance_info(&manager.get(&instance)?),
        InstanceCommand::Export { instance, output } => {
            let archive = manager.export(&instance, output.as_deref())?;
            println!("Exported '{}' to {}", instance, archive.display());
        }
        InstanceCommand::Import { file, name } => {
            let imported = manager.import(&file, name.as_deref())?;
            println!(
                "Imported '{}' ({}) from {}",
                imported.name(),
                imported.id(),
                file.display()
            );
        }
    }
    Ok(())
}

/// Resolve the latest release version id from the (cached) manifest.
async fn latest_release(dirs: &Directories) -> Result<String> {
    let manifest = load_manifest(dirs, false).await?;
    Ok(manifest.latest.release)
}

async fn cmd_instance_create(
    manager: &InstanceManager,
    name: &str,
    version: Option<String>,
) -> Result<()> {
    let version = match version {
        Some(v) => {
            if load_manifest(manager.dirs(), false)
                .await?
                .find(&v)
                .is_none()
            {
                anyhow::bail!(
                    "version '{v}' not found in manifest — try `mc-launcher version list`"
                );
            }
            v
        }
        None => latest_release(manager.dirs()).await?,
    };
    let instance = manager.create(name, &version)?;
    println!(
        "Created instance '{}' ({}) in {}",
        instance.name(),
        instance.id(),
        instance.dir().display()
    );
    Ok(())
}

fn print_instance_table(instances: &[Instance]) {
    println!("ID                 NAME       VERSION  LOADER     LAST PLAYED");
    for instance in instances {
        println!(
            "{:<18} {:<10} {:<8} {:<10} {}",
            instance.id(),
            instance.name(),
            instance.version(),
            loader_label(instance.loader()),
            instance.config.last_played_at.as_deref().unwrap_or("-")
        );
    }
}

fn print_instance_info(instance: &Instance) {
    println!("id:              {}", instance.id());
    println!("name:            {}", instance.name());
    println!("version:         {}", instance.version());
    println!("loader:          {}", loader_label(instance.loader()));
    println!("created:         {}", instance.config.created_at);
    println!(
        "last played:     {}",
        instance.config.last_played_at.as_deref().unwrap_or("-")
    );
    println!("instance dir:    {}", instance.dir().display());
    println!("game dir:        {}", instance.game_dir().display());
}

fn loader_label(loader: Option<&Loader>) -> String {
    loader.map_or_else(|| "-".to_owned(), |l| format!("{} {}", l.kind, l.version))
}

async fn load_manifest(dirs: &Directories, refresh: bool) -> Result<VersionManifest> {
    let cache = dirs.manifest_cache_path();
    version_manifest::load(
        &http_client(),
        MANIFEST_URL,
        &cache,
        version_manifest::MANIFEST_CACHE_TTL,
        refresh,
    )
    .await
    .context("failed to load the Mojang version manifest")
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("build HTTP client")
}
