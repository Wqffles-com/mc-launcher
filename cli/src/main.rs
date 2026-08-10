use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use mc_launcher_core::accounts::{Account, AccountManager};
use mc_launcher_core::assets::ProgressFn;
use mc_launcher_core::auth::{self, DeviceCode, DevicePoll};
use mc_launcher_core::dirs::Directories;
use mc_launcher_core::download::Progress;
use mc_launcher_core::instances::{Instance, InstanceManager, Loader, LoaderKind};
use mc_launcher_core::launch::{self, LaunchOptions, Player};
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
        /// Re-fetch the version metadata, ignoring caches
        #[arg(long)]
        refresh: bool,
    },
    /// Launch a Minecraft version (installing it first if needed)
    Launch {
        /// Version id (e.g. 1.21.4); defaults to the latest release
        version: Option<String>,
        /// Launch inside this instance instead (uses its version and game dir)
        #[arg(long)]
        instance: Option<String>,
        /// Sign in as this account (UUID or name); defaults to the most
        /// recently used account. Falls back to an offline profile when no
        /// account is signed in.
        #[arg(long)]
        account: Option<String>,
        /// Player name for the offline profile (ignored when --account is
        /// used or a signed-in account exists)
        #[arg(long, default_value = "Player")]
        username: String,
        /// JVM heap size, e.g. 2G
        #[arg(long)]
        memory: Option<String>,
        /// Path to a Java executable (or a directory containing `bin/java`);
        /// defaults to the best available runtime (system JVM of the required
        /// major, or an auto-downloaded one)
        #[arg(long)]
        java: Option<PathBuf>,
        /// Window width (enables custom resolution)
        #[arg(long)]
        width: Option<u32>,
        /// Window height (enables custom resolution)
        #[arg(long)]
        height: Option<u32>,
    },
    /// Sign in with a Microsoft account (device code flow)
    Login,
    /// Manage signed-in Microsoft accounts
    Account {
        #[command(subcommand)]
        command: AccountCommand,
    },
    /// Manage game instances
    Instance {
        #[command(subcommand)]
        command: Option<InstanceCommand>,
    },
    /// Version manifest & metadata
    Version(VersionArgs),
    /// Manage Java runtimes
    Java {
        #[command(subcommand)]
        command: JavaCommand,
    },
}

#[derive(Debug, Subcommand)]
enum JavaCommand {
    /// List detected system JVMs and cached managed runtimes
    List,
    /// Download and cache a managed Java runtime (auto-selected on launch)
    Install {
        /// Java major version (e.g. 8, 17, 21)
        major: u32,
    },
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

#[derive(Debug, Subcommand)]
enum AccountCommand {
    /// List signed-in accounts (most recently used first)
    List,
    /// Remove a signed-in account (by UUID or name)
    Remove {
        /// Account UUID or name
        account: String,
    },
    /// Switch the default account (most recently used is used by default)
    Use {
        /// Account UUID or name
        account: String,
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
        Command::Install { version, refresh } => cmd_install(version, refresh).await,
        Command::Launch {
            version,
            instance,
            account,
            username,
            memory,
            java,
            width,
            height,
        } => {
            cmd_launch(
                version, instance, account, &username, memory, java, width, height,
            )
            .await
        }
        Command::Login => cmd_login().await,
        Command::Account { command } => cmd_account(command),
        Command::Instance { command } => {
            cmd_instance(command.unwrap_or(InstanceCommand::List)).await
        }
        Command::Version(args) => cmd_version(args).await,
        Command::Java { command } => cmd_java(command).await,
    }
}

fn cmd_init() -> Result<()> {
    let dirs = Directories::discover().context("could not resolve the data directory")?;
    dirs.ensure_all()?;
    println!("Initialized {}", dirs.root().display());
    Ok(())
}

/// Resolve a version id to its manifest entry + cached version JSON.
async fn resolve_version(
    dirs: &Directories,
    id: &str,
    refresh: bool,
) -> Result<mc_launcher_core::version_manifest::VersionInfo> {
    let manifest = load_manifest(dirs, refresh).await?;
    manifest
        .find(id)
        .ok_or_else(|| {
            anyhow::anyhow!("version '{id}' not found in manifest — try `mc-launcher version list`")
        })
        .cloned()
}

async fn cmd_install(version: Option<String>, refresh: bool) -> Result<()> {
    let dirs = Directories::discover().context("could not resolve the data directory")?;
    dirs.ensure_all()?;
    let info = if let Some(id) = version {
        resolve_version(&dirs, &id, refresh).await?
    } else {
        let manifest = load_manifest(&dirs, false).await?;
        resolve_version(&dirs, &manifest.latest.release, false).await?
    };
    let version_json = launch::load_version_json(&dirs, &http_client(), &info, refresh)
        .await
        .context(format!("failed to load version JSON for '{}'", info.id))?;
    println!(
        "Installing Minecraft {} ({}) ...",
        version_json.id, version_json.kind
    );
    install_version(
        &dirs,
        &version_json,
        &scratch_game_dir(&dirs, &version_json.id),
    )
    .await?;
    println!("Installed {}", version_json.id);
    Ok(())
}

/// Run the Microsoft device code sign-in end to end and store the account.
async fn cmd_login() -> Result<()> {
    let dirs = Directories::discover().context("could not resolve the data directory")?;
    dirs.ensure_all()?;
    let client = http_client();
    let client_id = auth::client_id();
    println!("Requesting a Microsoft device code...");
    let code = auth::request_device_code(&client, &client_id)
        .await
        .context("failed to request a device code")?;
    let poll = auth::wait_for_device_approval(&client, &client_id, &code, print_device_code)
        .await
        .context("device code flow did not complete")?;
    let DevicePoll::Authorized {
        access_token,
        refresh_token,
    } = poll
    else {
        unreachable!("wait_for_device_approval only returns authorized or errors")
    };
    println!("Exchanging tokens...");
    let (mc, profile) = auth::complete_sign_in(&client, &access_token)
        .await
        .context("failed to complete the Xbox/Minecraft sign-in")?;
    let manager = AccountManager::new(dirs);
    let mut account = Account::new(&mc, &profile);
    account.refresh_token = Some(refresh_token);
    manager
        .save(&mut account)
        .context("failed to store the account")?;
    println!(
        "Signed in as {} ({}) — token stored {}",
        account.name,
        account.id,
        match account.token_storage.as_str() {
            "keyring" => "in the OS credential store",
            _ => "in the accounts directory (no OS keyring available)",
        }
    );
    Ok(())
}

fn print_device_code(code: &DeviceCode) {
    println!("Go to {} and enter:", code.verification_uri);
    println!();
    println!("    {}", code.user_code);
    println!();
    if let Some(uri) = &code.verification_uri_complete {
        println!("or open this link directly: {uri}");
    }
    if let Some(message) = &code.message {
        println!("{message}");
    }
    println!(
        "Waiting for approval (code expires in {}s)...",
        code.expires_in
    );
}

fn cmd_account(command: AccountCommand) -> Result<()> {
    let dirs = Directories::discover().context("could not resolve the data directory")?;
    dirs.ensure_all()?;
    let manager = AccountManager::new(dirs);
    match command {
        AccountCommand::List => {
            let accounts = manager.list()?;
            if accounts.is_empty() {
                println!(
                    "No accounts signed in — use `mc-launcher login` to sign in with Microsoft"
                );
                return Ok(());
            }
            println!(
                "NAME       UUID                                  EXPIRES             STORAGE"
            );
            for account in &accounts {
                println!(
                    "{:<10} {:<36} {:<20} {}",
                    account.name,
                    account.id,
                    account.expires_at.as_deref().unwrap_or("-"),
                    account.token_storage
                );
            }
        }
        AccountCommand::Remove { account } => {
            manager.remove(&account)?;
            println!("Removed account '{account}'");
        }
        AccountCommand::Use { account } => {
            let used = manager.get(&account)?;
            manager.touch(&used.id)?;
            println!("Switched default account to {} ({})", used.name, used.id);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_launch(
    version: Option<String>,
    instance: Option<String>,
    account: Option<String>,
    username: &str,
    memory: Option<String>,
    java: Option<PathBuf>,
    width: Option<u32>,
    height: Option<u32>,
) -> Result<()> {
    let resolution = match (width, height) {
        (Some(width), Some(height)) => Some((width, height)),
        (None, None) => None,
        _ => anyhow::bail!("--width and --height must be given together"),
    };
    let dirs = Directories::discover().context("could not resolve the data directory")?;
    dirs.ensure_all()?;
    let manager = InstanceManager::new(dirs.clone());

    let (version_id, game_dir, touch_id) = if let Some(name) = instance {
        if version.is_some() {
            anyhow::bail!("a version id cannot be combined with --instance");
        }
        let instance = manager.get(&name)?;
        (
            instance.version().to_owned(),
            instance.game_dir(),
            Some(instance.id().to_owned()),
        )
    } else {
        let version_id = match version {
            Some(id) => id,
            None => latest_release(&dirs).await?,
        };
        let game_dir = scratch_game_dir(&dirs, &version_id);
        (version_id, game_dir, None)
    };
    std::fs::create_dir_all(&game_dir)?;

    let info = resolve_version(&dirs, &version_id, false).await?;
    let version_json = launch::load_version_json(&dirs, &http_client(), &info, false)
        .await
        .context(format!("failed to load version JSON for '{version_id}'"))?;

    println!(
        "Preparing Minecraft {version_id} ({}) ...",
        version_json.kind
    );
    let player = resolve_player(&dirs, account.as_deref(), username)
        .await
        .context("could not resolve a player profile")?;
    if player.user_type == "msa" {
        println!("Launching as {} ({})", player.name, player.uuid);
    } else {
        println!(
            "No signed-in account — launching with an offline profile as '{}' \
             (use `mc-launcher login`)",
            player.name
        );
    }
    let options = LaunchOptions {
        game_dir: game_dir.clone(),
        java,
        memory,
        resolution,
        on_output: Some(Box::new(|line| {
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(stdout, "{line}");
        })),
    };
    let progress: Option<ProgressFn> = Some(Arc::new(print_progress));
    let outcome = launch::launch(
        &dirs,
        &download_client(),
        &version_json,
        &player,
        options,
        progress,
    )
    .await
    .context("launch failed")?;

    if let Some(id) = touch_id {
        manager.touch(&id)?;
    }
    println!(
        "Game exited with code {} (log: {})",
        outcome
            .exit
            .code()
            .map_or_else(|| "?".to_owned(), |code| code.to_string()),
        outcome.log_file.display()
    );
    if !outcome.exit.success() {
        std::process::exit(outcome.exit.code().unwrap_or(1));
    }
    Ok(())
}

/// Resolve the player profile for a launch: the explicitly selected account,
/// else the default (most recently used) account, else an offline profile.
/// Accounts with an expired Minecraft token are refreshed automatically.
async fn resolve_player(
    dirs: &Directories,
    selector: Option<&str>,
    offline_name: &str,
) -> Result<Player> {
    let manager = AccountManager::new(dirs.clone());
    let account = match selector {
        Some(sel) => Some(manager.get(sel)?),
        // The offline path must not depend on the accounts directory being
        // healthy: a corrupt account file falls back to offline with a
        // warning instead of failing the launch.
        None => match manager.default() {
            Ok(account) => account,
            Err(e) => {
                eprintln!("warning: could not read signed-in accounts ({e}); launching offline");
                None
            }
        },
    };
    let Some(mut account) = account else {
        return Ok(Player::offline(offline_name));
    };
    if account.access_token_expired() {
        println!("Refreshing tokens for {} ...", account.name);
        account = manager
            .refresh(&http_client(), &account)
            .await
            .context("failed to refresh the account tokens")?;
    }
    manager.touch(&account.id)?;
    Ok(Player::microsoft(
        &account.name,
        &account.id,
        &account.access_token,
    ))
}

/// Install (or verify) all artifacts of a version, printing progress.
async fn install_version(
    dirs: &Directories,
    version_json: &mc_launcher_core::version_json::VersionJson,
    game_dir: &std::path::Path,
) -> Result<()> {
    let progress: Option<ProgressFn> = Some(Arc::new(print_progress));
    launch::install(dirs, &download_client(), version_json, game_dir, progress)
        .await
        .context("install failed")?;
    Ok(())
}

/// The scratch game directory used for bare (non-instance) launches.
fn scratch_game_dir(dirs: &Directories, version_id: &str) -> PathBuf {
    dirs.root().join("launch").join(version_id)
}

fn print_progress(progress: Progress) {
    let mut stdout = std::io::stdout().lock();
    match progress {
        Progress::File { name, done, total } => {
            let percent = done.saturating_mul(100).checked_div(total).unwrap_or(0);
            let _ = write!(stdout, "\r{name}: {percent}% ({done} / {total} bytes)");
        }
        Progress::FileDone { name, fresh } => {
            let _ = writeln!(
                stdout,
                "\r{name}: {}",
                if fresh { "already present" } else { "done" }
            );
        }
        Progress::BatchDone { name, done, total } => {
            let _ = writeln!(stdout, "[{done}/{total}] {name}");
        }
    }
    let _ = stdout.flush();
}

async fn cmd_java(command: JavaCommand) -> Result<()> {
    let dirs = Directories::discover().context("could not resolve the data directory")?;
    dirs.ensure_all()?;
    match command {
        JavaCommand::List => {
            let system = mc_launcher_core::java::detect_system();
            println!("SYSTEM JVMS");
            if system.is_empty() {
                println!("  (none found)");
            }
            for runtime in &system {
                println!("  Java {}: {}", runtime.major, runtime.home.display());
            }
            let managed = mc_launcher_core::java::managed_runtimes(&dirs);
            println!("MANAGED RUNTIMES");
            if managed.is_empty() {
                println!("  (none — `mc-launcher java install <major>` to download one)");
            }
            for runtime in managed {
                println!("  Java {}: {}", runtime.major, runtime.home.display());
            }
        }
        JavaCommand::Install { major } => {
            if let Some(runtime) = mc_launcher_core::java::managed_runtime(&dirs, major) {
                println!(
                    "Java {} already installed at {}",
                    runtime.major,
                    runtime.home.display()
                );
                return Ok(());
            }
            let component = mc_launcher_core::java::component_for_major(major);
            println!("Downloading Java {major} ...");
            let progress: Option<ProgressFn> = Some(Arc::new(print_progress));
            let runtime = mc_launcher_core::java::ensure_runtime(
                &dirs,
                &download_client(),
                major,
                component,
                progress,
            )
            .await
            .context(format!("failed to install Java {major}"))?;
            println!(
                "Installed Java {} at {}",
                runtime.major,
                runtime.home.display()
            );
        }
    }
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
                version.asset_index.as_ref().map_or("-", |a| a.id.as_str()),
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

/// A client with a long timeout for large downloads.
fn download_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_mins(10))
        .build()
        .expect("build download client")
}
