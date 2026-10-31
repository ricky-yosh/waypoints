//! Human- and agent-facing commands for the local Waypoints backend.

use std::env;
use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use waypoints_backend::{Client, Config};

#[derive(Parser)]
#[command(
    name = "waypoints",
    version,
    about = "Local workflow storage and backend"
)]
struct Args {
    /// Database file (defaults to the user state directory).
    #[arg(long, global = true)]
    database: Option<PathBuf>,
    /// Local Unix socket (defaults alongside the database).
    #[arg(long, global = true)]
    socket: Option<PathBuf>,
    /// Print machine-readable JSON.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the backend in the foreground (normally started automatically).
    Serve,
    /// Create or list saved workflows.
    Workflow {
        #[command(subcommand)]
        command: WorkflowCommand,
    },
    /// Inspect or stop an existing backend without starting one.
    Backend {
        #[command(subcommand)]
        command: BackendCommand,
    },
}

#[derive(Subcommand)]
enum WorkflowCommand {
    Create {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        name: String,
    },
    List {
        #[arg(long)]
        project: PathBuf,
    },
}

#[derive(Subcommand)]
enum BackendCommand {
    Status,
    Stop,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(run(args))
}

async fn run(args: Args) -> anyhow::Result<()> {
    let database = match args.database {
        Some(path) => path,
        None => state_directory()?.join("workflows.db"),
    };
    let socket = args
        .socket
        .unwrap_or_else(|| database.with_extension("sock"));
    let config = Config::new(&database, &socket)?;
    match args.command {
        Command::Serve => waypoints_backend::serve(&config).await?,
        Command::Backend { command } => {
            let client = Client::connect(&config).await?;
            match command {
                BackendCommand::Status => {
                    if args.json {
                        println!("{}", serde_json::to_string(client.info())?);
                    } else {
                        println!(
                            "Backend PID {} · {}",
                            client.info().pid,
                            client.info().database.display()
                        );
                    }
                }
                BackendCommand::Stop => {
                    client.stop().await?;
                    if args.json {
                        println!("{{\"stopped\":true}}");
                    } else {
                        println!("Backend stopped");
                    }
                }
            }
        }
        Command::Workflow { command } => {
            let client = Client::connect_or_start(&config, &env::current_exe()?).await?;
            match command {
                WorkflowCommand::Create { project, name } => {
                    let workflow = client.create_workflow(&project, &name).await?;
                    if args.json {
                        println!("{}", serde_json::to_string(&workflow)?);
                    } else {
                        println!("Created workflow {}: {}", workflow.id, workflow.name);
                    }
                }
                WorkflowCommand::List { project } => {
                    let workflows = client.list_workflows(&project).await?;
                    if args.json {
                        println!("{}", serde_json::to_string(&workflows)?);
                    } else {
                        for workflow in workflows {
                            println!("{}\t{}", workflow.id, workflow.name);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn state_directory() -> anyhow::Result<PathBuf> {
    if let Some(path) = env::var_os("XDG_STATE_HOME").filter(|path| !path.is_empty()) {
        let path = PathBuf::from(path);
        anyhow::ensure!(path.is_absolute(), "XDG_STATE_HOME must be absolute");
        return Ok(path.join("waypoints"));
    }
    let home = PathBuf::from(env::var_os("HOME").context("HOME is not set; supply --database")?);
    #[cfg(target_os = "macos")]
    let path = home.join("Library/Application Support/waypoints");
    #[cfg(not(target_os = "macos"))]
    let path = home.join(".local/state/waypoints");
    Ok(path)
}
