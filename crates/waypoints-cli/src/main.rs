//! Human- and agent-facing commands for the local Waypoints backend.

use std::env;
use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use waypoints_backend::{Client, Config};
use waypoints_core::jj::ChangeState;
use waypoints_core::storage::{Node, NodeChange};

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
    /// Create, list, or inspect saved workflows.
    Workflow {
        #[command(subcommand)]
        command: WorkflowCommand,
    },
    /// Create, read, or edit saved prompt nodes.
    Node {
        #[command(subcommand)]
        command: NodeCommand,
    },
    /// Save directed workflow connections (not jj ancestry or execution).
    Connection {
        #[command(subcommand)]
        command: ConnectionCommand,
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
    Graph {
        #[arg(long)]
        workflow: i64,
    },
}

#[derive(Subcommand)]
enum NodeCommand {
    Create {
        #[arg(long)]
        workflow: i64,
        #[arg(long)]
        prompt: String,
    },
    Get {
        #[arg(long)]
        workflow: i64,
        #[arg(long)]
        node: i64,
    },
    Update {
        #[arg(long)]
        workflow: i64,
        #[arg(long)]
        node: i64,
        #[arg(long)]
        prompt: String,
    },
    /// Associate an existing jj change by change ID (or unique prefix), not a revset.
    Associate {
        #[arg(long)]
        workflow: i64,
        #[arg(long)]
        node: i64,
        #[arg(long)]
        change: String,
    },
    /// Create an empty jj change on an explicit base, without switching the working copy.
    CreateChange {
        #[arg(long)]
        workflow: i64,
        #[arg(long)]
        node: i64,
        /// Change ID, commit ID, unique prefix, or root(); not an arbitrary revset.
        #[arg(long)]
        base: String,
    },
    /// Query the node's current native revision without snapshotting the working copy.
    Refresh {
        #[arg(long)]
        workflow: i64,
        #[arg(long)]
        node: i64,
    },
}

#[derive(Subcommand)]
enum ConnectionCommand {
    Create {
        #[arg(long)]
        workflow: i64,
        #[arg(long)]
        source: i64,
        #[arg(long)]
        target: i64,
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
                WorkflowCommand::Graph { workflow } => {
                    let graph = client.workflow_graph(workflow).await?;
                    if args.json {
                        println!("{}", serde_json::to_string(&graph)?);
                    } else {
                        println!("Workflow {}: {}", graph.workflow.id, graph.workflow.name);
                        for node in graph.nodes {
                            println!("Node {}: {}", node.id, node.prompt);
                        }
                        for connection in graph.connections {
                            println!("{} -> {}", connection.source_id, connection.target_id);
                        }
                    }
                }
            }
        }
        Command::Node { command } => {
            let client = Client::connect_or_start(&config, &env::current_exe()?).await?;
            match command {
                NodeCommand::Associate {
                    workflow,
                    node,
                    change,
                } => {
                    let status = client.associate_node(workflow, node, &change).await?;
                    print_node_change(status, args.json)?;
                }
                NodeCommand::CreateChange {
                    workflow,
                    node,
                    base,
                } => {
                    let status = client.create_node_change(workflow, node, &base).await?;
                    print_node_change(status, args.json)?;
                }
                NodeCommand::Refresh { workflow, node } => {
                    let status = client.refresh_node(workflow, node).await?;
                    print_node_change(status, args.json)?;
                }
                NodeCommand::Create { workflow, prompt } => {
                    print_node(client.create_node(workflow, &prompt).await?, args.json)?;
                }
                NodeCommand::Get { workflow, node } => {
                    print_node(client.node(workflow, node).await?, args.json)?;
                }
                NodeCommand::Update {
                    workflow,
                    node,
                    prompt,
                } => {
                    print_node(
                        client.update_node(workflow, node, &prompt).await?,
                        args.json,
                    )?;
                }
            }
        }
        Command::Connection { command } => {
            let client = Client::connect_or_start(&config, &env::current_exe()?).await?;
            match command {
                ConnectionCommand::Create {
                    workflow,
                    source,
                    target,
                } => {
                    let connection = client.connect_nodes(workflow, source, target).await?;
                    if args.json {
                        println!("{}", serde_json::to_string(&connection)?);
                    } else {
                        println!(
                            "Connected {} -> {} in workflow {}",
                            connection.source_id, connection.target_id, connection.workflow_id
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn print_node(node: Node, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string(&node)?);
    } else {
        println!(
            "Node {} (workflow {}): {}",
            node.id, node.workflow_id, node.prompt
        );
    }
    Ok(())
}

fn print_node_change(value: NodeChange, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string(&value)?);
    } else {
        print_node(value.node, false)?;
        if let Some(association) = value.association {
            println!(
                "Change: {} · {}",
                association.change_id,
                association.repository_path.display()
            );
        }
        match value.state {
            ChangeState::Current { revision } => {
                println!("jj status: current");
                println!("Commit: {}", revision.commit_id);
                println!("Conflicted: {}", revision.conflicted);
                println!("Description: {}", revision.description);
            }
            ChangeState::Divergent { .. } => println!(
                "jj status: divergent\nMultiple native revisions; no revision selected. Use --json to inspect candidates."
            ),
            ChangeState::Missing => println!(
                "jj status: missing\nSaved node retained. No visible revision found; nothing recreated."
            ),
            ChangeState::Unassociated => println!("jj status: unassociated"),
            _ => println!("jj status: unknown"),
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
