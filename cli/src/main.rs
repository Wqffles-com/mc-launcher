use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use mc_launcher_core::dirs::Directories;
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
    /// Create a new instance
    Create { name: String },
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
        Command::Instance { .. } => {
            anyhow::bail!("not implemented yet — instances land in the next phase (EPIC-65jcv)")
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
