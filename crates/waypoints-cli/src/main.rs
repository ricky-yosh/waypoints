//! Human- and agent-facing commands for the local Waypoints backend.

use std::env;
use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use waypoints_backend::{Client, Config};
use waypoints_core::jj::ChangeState;
use waypoints_core::run::RecoveryConfirmation;
use waypoints_core::storage::{HistoryStatus, Node, NodeChange};
use waypoints_core::workflow::BaseSource;
use waypoints_core::workspace::{NodeWorkspace, WorkspaceState};

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
    /// Inspect or undo/redo saved workflow metadata; never change code or agent memory.
    History {
        #[command(subcommand)]
        command: HistoryCommand,
    },
    /// Start or control backend-owned ACP runs; output polling never grants permissions.
    Run {
        #[command(subcommand)]
        command: RunCommand,
    },
}

#[derive(Subcommand)]
enum HistoryCommand {
    /// Show the next undo and redo actions across this project's workflows.
    Status {
        #[arg(long)]
        project: PathBuf,
    },
    /// Undo the pending action ID returned by history status.
    Undo {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        expected: i64,
    },
    /// Redo the pending action ID returned by history status.
    Redo {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        expected: i64,
    },
}

#[derive(Subcommand)]
enum RunCommand {
    /// Capture a node prompt and start an ACP executable in its verified workspace.
    Start {
        #[arg(long)]
        workflow: i64,
        #[arg(long)]
        node: i64,
        /// Explicit installed executable path; no shell command or arguments.
        #[arg(long)]
        executable: PathBuf,
    },
    /// Load a stopped run's saved session and send a follow-up, without changing the node prompt.
    Continue {
        /// Latest run for the session to continue; no conversation fork or rollback.
        #[arg(long)]
        run: i64,
        #[arg(long)]
        prompt: String,
    },
    List {
        #[arg(long)]
        workflow: i64,
        #[arg(long)]
        node: i64,
    },
    Status {
        #[arg(long)]
        run: i64,
    },
    /// Inspect interrupted-run metadata and read-only workspace checks; never release it.
    Recovery {
        #[arg(long)]
        run: i64,
    },
    /// Release an interrupted reservation after fresh checks and explicit confirmation.
    Recover {
        #[arg(long)]
        run: i64,
        /// Attest the old adapter, tools, and native jj writers stopped; no PID is signalled.
        #[arg(long, required = true)]
        confirm_processes_stopped: bool,
    },
    /// Poll live updates once; use the returned next cursor for subsequent polls.
    Events {
        #[arg(long)]
        run: i64,
        #[arg(long, default_value_t = 0)]
        after: u64,
    },
    Cancel {
        #[arg(long)]
        run: i64,
    },
    /// Explicitly answer a pending permission with its advertised single-use option ID.
    Permission {
        #[arg(long)]
        run: i64,
        #[arg(long)]
        permission: u64,
        #[arg(long)]
        option: String,
    },
}

#[derive(Subcommand)]
enum WorkflowCommand {
    Create {
        #[arg(long)]
        project: PathBuf,
        #[arg(long)]
        name: String,
        /// Start from an exact local jj bookmark name, e.g. main or feature/auth.
        #[arg(long, conflicts_with = "base_revision")]
        base: Option<String>,
        /// Start from a commit/change ID or unique prefix instead of a bookmark.
        #[arg(long, conflicts_with = "base")]
        base_revision: Option<String>,
    },
    /// Inspect saved workflow metadata and its fixed base without querying jj.
    Get {
        #[arg(long)]
        workflow: i64,
    },
    /// Select an initial base for an unbased workflow; does not rebase existing nodes.
    SetBase {
        #[arg(long)]
        workflow: i64,
        #[arg(
            long,
            conflicts_with = "base_revision",
            required_unless_present = "base_revision"
        )]
        base: Option<String>,
        #[arg(long, conflicts_with = "base", required_unless_present = "base")]
        base_revision: Option<String>,
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
    /// Allocate or inspect a dedicated native workspace for this node's change.
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommand,
    },
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
enum WorkspaceCommand {
    /// Verify checkout completion and mark a saved allocation ready; never alter files or jj history.
    Verify {
        #[arg(long)]
        workflow: i64,
        #[arg(long)]
        node: i64,
    },
    /// Create a workspace in a new directory; repeat requests report the saved allocation.
    Create {
        #[arg(long)]
        workflow: i64,
        #[arg(long)]
        node: i64,
        /// New directory, relative to this CLI's current directory or absolute.
        #[arg(long)]
        directory: PathBuf,
    },
    /// Inspect the saved workspace without snapshotting, switching, or repairing it.
    Status {
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
        Command::Run { command } => {
            // Explicit starts/continuations and durable queries may launch the backend.
            // Live controls must not conceal a restart.
            let client = if matches!(
                command,
                RunCommand::Start { .. }
                    | RunCommand::Continue { .. }
                    | RunCommand::List { .. }
                    | RunCommand::Status { .. }
                    | RunCommand::Recovery { .. }
            ) {
                Client::connect_or_start(&config, &env::current_exe()?).await?
            } else {
                Client::connect(&config).await?
            };
            let value = match command {
                RunCommand::Start {
                    workflow,
                    node,
                    executable,
                } => {
                    let executable = env::current_dir()?.join(executable);
                    serde_json::to_value(client.start_run(workflow, node, &executable).await?)?
                }
                RunCommand::Continue { run, prompt } => {
                    serde_json::to_value(client.continue_run(run, &prompt).await?)?
                }
                RunCommand::List { workflow, node } => {
                    serde_json::to_value(client.node_runs(workflow, node).await?)?
                }
                RunCommand::Status { run } => serde_json::to_value(client.run_status(run).await?)?,
                RunCommand::Recovery { run } => {
                    serde_json::to_value(client.inspect_run_recovery(run).await?)?
                }
                RunCommand::Recover {
                    run,
                    confirm_processes_stopped,
                } => {
                    anyhow::ensure!(
                        confirm_processes_stopped,
                        "recovery requires --confirm-processes-stopped"
                    );
                    serde_json::to_value(
                        client
                            .recover_run(run, RecoveryConfirmation::ProcessesStopped)
                            .await?,
                    )?
                }
                RunCommand::Events { run, after } => {
                    serde_json::to_value(client.run_events(run, after).await?)?
                }
                RunCommand::Cancel { run } => serde_json::to_value(client.cancel_run(run).await?)?,
                RunCommand::Permission {
                    run,
                    permission,
                    option,
                } => serde_json::to_value(
                    client
                        .respond_to_permission(run, permission, &option)
                        .await?,
                )?,
            };
            if args.json {
                println!("{}", serde_json::to_string(&value)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
        }
        Command::History { command } => {
            let client = Client::connect_or_start(&config, &env::current_exe()?).await?;
            let status = match command {
                HistoryCommand::Status { project } => client.history_status(&project).await?,
                HistoryCommand::Undo { project, expected } => {
                    client.undo(&project, expected).await?
                }
                HistoryCommand::Redo { project, expected } => {
                    client.redo(&project, expected).await?
                }
            };
            print_history(status, args.json)?;
        }
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
                WorkflowCommand::Create {
                    project,
                    name,
                    base,
                    base_revision,
                } => {
                    let workflow = match base_source(base, base_revision) {
                        Some(source) => {
                            client
                                .create_workflow_from(&project, &name, &source)
                                .await?
                        }
                        None => client.create_workflow(&project, &name).await?,
                    };
                    if args.json {
                        println!("{}", serde_json::to_string(&workflow)?);
                    } else {
                        println!("Created workflow {}: {}", workflow.id, workflow.name);
                    }
                }
                WorkflowCommand::Get { workflow } => {
                    let workflow = client.workflow(workflow).await?;
                    println!(
                        "{}",
                        if args.json {
                            serde_json::to_string(&workflow)?
                        } else {
                            serde_json::to_string_pretty(&workflow)?
                        }
                    );
                }
                WorkflowCommand::SetBase {
                    workflow,
                    base,
                    base_revision,
                } => {
                    let source = base_source(base, base_revision)
                        .context("select --base or --base-revision")?;
                    let workflow = client.set_workflow_base(workflow, &source).await?;
                    println!(
                        "{}",
                        if args.json {
                            serde_json::to_string(&workflow)?
                        } else {
                            serde_json::to_string_pretty(&workflow)?
                        }
                    );
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
                NodeCommand::Workspace { command } => {
                    let value = match command {
                        WorkspaceCommand::Create {
                            workflow,
                            node,
                            directory,
                        } => {
                            let directory = env::current_dir()?.join(directory);
                            client
                                .create_node_workspace(workflow, node, &directory)
                                .await?
                        }
                        WorkspaceCommand::Verify { workflow, node } => {
                            client.verify_node_workspace(workflow, node).await?
                        }
                        WorkspaceCommand::Status { workflow, node } => {
                            client.node_workspace(workflow, node).await?
                        }
                    };
                    print_node_workspace(value, args.json)?;
                }
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

fn base_source(bookmark: Option<String>, revision: Option<String>) -> Option<BaseSource> {
    bookmark
        .map(|name| BaseSource::Bookmark { name })
        .or_else(|| revision.map(|id| BaseSource::Revision { id }))
}

fn print_history(status: HistoryStatus, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string(&status)?);
    } else {
        println!("Project: {}", status.project_path.display());
        for (direction, entry) in [("Undo", status.undo), ("Redo", status.redo)] {
            match entry {
                Some(entry) => println!(
                    "{direction}: {} (edit {}, workflow {})",
                    entry.label, entry.id, entry.workflow_id
                ),
                None => println!("{direction}: unavailable"),
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

fn print_node_workspace(value: NodeWorkspace, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string(&value)?);
    } else {
        print_node(value.node, false)?;
        if let Some(workspace) = value.workspace {
            println!("Workspace: {}", workspace.name);
            println!("Directory: {}", workspace.directory.display());
            println!("Change: {}", workspace.change_id);
        }
        match value.state {
            WorkspaceState::Unallocated => println!("Workspace status: unallocated"),
            WorkspaceState::Ready { revision } => {
                println!("Workspace status: ready (recorded jj state; files not snapshotted)");
                println!("Commit: {}", revision.commit_id);
            }
            WorkspaceState::Incomplete => println!(
                "Workspace status: incomplete\nIf checkout completed, use node workspace verify to acknowledge it. Otherwise inspect native jj state before manual recovery. No files were removed; creation will not be retried."
            ),
            WorkspaceState::MissingDirectory => println!(
                "Workspace status: missing_directory\nSaved allocation retained; nothing recreated."
            ),
            WorkspaceState::Mismatched { detail } => {
                println!("Workspace status: mismatched\n{detail}")
            }
            _ => println!("Workspace status: unknown"),
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
