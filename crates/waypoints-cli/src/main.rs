//! Human- and agent-facing commands for the local Waypoints backend.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use waypoints_backend::{Client, Config};
use waypoints_core::jj::ChangeState;
use waypoints_core::run::RecoveryConfirmation;
use waypoints_core::storage::{HistoryStatus, Node, NodeChange, UndoBoundaryAcknowledgement};
use waypoints_core::workflow::{BaseSource, NodeParent};
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
    /// Compare direct upstream revisions with those observed before this run.
    Inputs {
        #[arg(long)]
        run: i64,
    },
    /// Review the exact native snapshot saved by a run, without updating any checkout.
    Review {
        #[arg(long)]
        run: i64,
    },
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
    /// Rename a workflow; undoable metadata only, never changes code or runs.
    Rename {
        #[arg(long)]
        workflow: i64,
        #[arg(long)]
        name: String,
    },
    /// Explicitly connect every legacy prompt-only graph root to the saved workflow base.
    AdoptRoots {
        #[arg(long)]
        workflow: i64,
        #[arg(long, required = true, num_args = 1.., value_delimiter = ',')]
        base_nodes: Vec<i64>,
    },
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
    /// Review the current recorded native commit, excluding unsnapshotted workspace edits.
    Review {
        #[arg(long)]
        workflow: i64,
        #[arg(long)]
        node: i64,
    },
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
        /// Build directly on this workflow's fixed base.
        #[arg(long, conflicts_with = "parent")]
        from_base: bool,
        /// Build on upstream nodes in this workflow; repeat --parent to combine code.
        #[arg(long, conflicts_with = "from_base", value_delimiter = ',')]
        parent: Vec<i64>,
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
    /// Create an empty jj change from the saved dependency, without switching the working copy.
    CreateChange {
        #[arg(long)]
        workflow: i64,
        #[arg(long)]
        node: i64,
        /// Legacy workflow base; in based workflows it must match the saved dependency.
        #[arg(long)]
        base: Option<String>,
    },
    /// Move a prompt-only subtree without changing native code; supports metadata Undo.
    MovePrompt {
        #[arg(long)]
        workflow: i64,
        #[arg(long)]
        node: i64,
        #[arg(long, required_unless_present = "parent", conflicts_with = "parent")]
        from_base: bool,
        #[arg(
            long,
            required_unless_present = "from_base",
            conflicts_with = "from_base"
        )]
        parent: Option<i64>,
    },
    /// Delete a prompt-only node and reconnect children; supports metadata Undo.
    DeletePrompt {
        #[arg(long)]
        workflow: i64,
        #[arg(long)]
        node: i64,
    },
    /// Rebase this node and descendants onto a replacement dependency.
    Reparent {
        #[arg(long)]
        workflow: i64,
        #[arg(long)]
        node: i64,
        #[arg(long, required_unless_present = "parent", conflicts_with = "parent")]
        from_base: bool,
        #[arg(
            long,
            required_unless_present = "from_base",
            conflicts_with = "from_base"
        )]
        parent: Option<i64>,
        /// Accept that workflow Undo cannot cross this code-changing operation.
        #[arg(long)]
        accept_undo_boundary: bool,
    },
    /// Continue an abandoned deletion by deregistering its workspace, keeping all files.
    ResumeDeletion {
        #[arg(long)]
        operation: i64,
        /// Attest old native processes and other writers have stopped.
        #[arg(long, required = true)]
        confirm_processes_stopped: bool,
    },
    /// Abandon a native node and reconnect children; deregister its saved workspace, keeping all files.
    Delete {
        #[arg(long)]
        workflow: i64,
        #[arg(long)]
        node: i64,
        /// Accept that deletion changes native code and workflow Undo cannot cross it.
        #[arg(long)]
        accept_undo_boundary: bool,
    },
    /// Inspect durable dependency operations; unresolved outcomes must not be retried.
    Operations {
        #[arg(long)]
        workflow: i64,
    },
    /// Inspect a dependency operation without releasing its reservation or changing jj.
    #[command(name = "operation-recovery", visible_alias = "reparent-recovery")]
    ReparentRecovery {
        #[arg(long)]
        operation: i64,
    },
    /// Resolve a clearly applied/unapplied dependency operation; never rerun jj or undo code.
    #[command(name = "recover-operation", visible_alias = "recover-reparent")]
    RecoverReparent {
        #[arg(long)]
        operation: i64,
        /// Attest that all old native processes and other writers have stopped.
        #[arg(long)]
        confirm_processes_stopped: bool,
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

fn main() -> ExitCode {
    match run_cli() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:?}");
            if error
                .downcast_ref::<waypoints_backend::BackendError>()
                .is_some_and(|error| error.is_ownership_unavailable())
            {
                ExitCode::from(waypoints_backend::OWNERSHIP_UNAVAILABLE_EXIT_CODE)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

fn run_cli() -> anyhow::Result<()> {
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
                    | RunCommand::Inputs { .. }
                    | RunCommand::Review { .. }
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
                RunCommand::Review { run } => {
                    print_review(client.review_run(run).await?, args.json)?;
                    return Ok(());
                }
                RunCommand::Inputs { run } => {
                    let status = client.run_upstream_status(run).await?;
                    if args.json {
                        println!("{}", serde_json::to_string(&status)?);
                    } else {
                        print_upstream_status(&status);
                    }
                    return Ok(());
                }
                RunCommand::Status { run } => {
                    let status = client.run_status(run).await?;
                    if !args.json {
                        println!("{}", serde_json::to_string_pretty(&status)?);
                        match client.run_upstream_status(run).await {
                            Ok(inputs) => print_upstream_status(&inputs),
                            Err(error) => println!("Upstream comparison unavailable: {error}"),
                        }
                        return Ok(());
                    }
                    serde_json::to_value(status)?
                }
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
                WorkflowCommand::Rename { workflow, name } => {
                    let workflow = client.rename_workflow(workflow, &name).await?;
                    if args.json {
                        println!("{}", serde_json::to_string(&workflow)?);
                    } else {
                        println!("Renamed workflow {}: {}", workflow.id, workflow.name);
                    }
                }
                WorkflowCommand::AdoptRoots {
                    workflow,
                    base_nodes,
                } => {
                    let graph = client.adopt_workflow_roots(workflow, &base_nodes).await?;
                    println!(
                        "{}",
                        if args.json {
                            serde_json::to_string(&graph)?
                        } else {
                            serde_json::to_string_pretty(&graph)?
                        }
                    );
                }
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
                        println!(
                            "Connectivity: {}",
                            serde_json::to_string(&graph.connectivity)?
                        );
                        for id in graph.base_nodes {
                            println!("Base -> {id}");
                        }
                        for node in graph.nodes {
                            println!("Node {}: {}", node.id, node.prompt);
                        }
                        for connection in graph.connections {
                            println!("{} -> {}", connection.source_id, connection.target_id);
                        }
                        if let Some(id) = graph.unresolved_dependency_operation {
                            println!(
                                "UNRESOLVED dependency operation {id}: saved edges may not match jj; inspect node operations."
                            );
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
                    let status = match base {
                        Some(base) => client.create_node_change(workflow, node, &base).await?,
                        None => {
                            client
                                .create_node_change_from_dependency(workflow, node)
                                .await?
                        }
                    };
                    print_node_change(status, args.json)?;
                }
                NodeCommand::MovePrompt {
                    workflow,
                    node,
                    from_base,
                    parent,
                } => {
                    let parent = if from_base {
                        NodeParent::WorkflowBase {}
                    } else {
                        NodeParent::Node {
                            node_id: parent.context("select --parent or --from-base")?,
                        }
                    };
                    let graph = client.move_prompt_node(workflow, node, &parent).await?;
                    println!(
                        "{}",
                        if args.json {
                            serde_json::to_string(&graph)?
                        } else {
                            serde_json::to_string_pretty(&graph)?
                        }
                    );
                }
                NodeCommand::DeletePrompt { workflow, node } => {
                    let graph = client.delete_prompt_node(workflow, node).await?;
                    println!(
                        "{}",
                        if args.json {
                            serde_json::to_string(&graph)?
                        } else {
                            serde_json::to_string_pretty(&graph)?
                        }
                    );
                }
                NodeCommand::Reparent {
                    workflow,
                    node,
                    from_base,
                    parent,
                    accept_undo_boundary,
                } => {
                    let parent = if from_base {
                        NodeParent::WorkflowBase {}
                    } else {
                        NodeParent::Node {
                            node_id: parent.context("select --parent or --from-base")?,
                        }
                    };
                    let operation = client
                        .reparent_node(
                            workflow,
                            node,
                            &parent,
                            accept_undo_boundary
                                .then_some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
                        )
                        .await?;
                    if args.json {
                        println!("{}", serde_json::to_string(&operation)?);
                    } else {
                        println!(
                            "Dependency operation {} completed; workflow Undo starts at this boundary.",
                            operation.id
                        );
                        for revision in &operation.after {
                            println!(
                                "{} {}{}",
                                revision.change_id,
                                revision.commit_id,
                                if revision.conflicted { " CONFLICT" } else { "" }
                            );
                        }
                        println!("{}", operation.detail);
                    }
                }
                NodeCommand::ResumeDeletion {
                    operation,
                    confirm_processes_stopped,
                } => {
                    let result = client
                        .resume_node_deletion(
                            operation,
                            confirm_processes_stopped
                                .then_some(RecoveryConfirmation::ProcessesStopped),
                        )
                        .await?;
                    println!(
                        "{}",
                        if args.json {
                            serde_json::to_string(&result)?
                        } else {
                            serde_json::to_string_pretty(&result)?
                        }
                    );
                }
                NodeCommand::Delete {
                    workflow,
                    node,
                    accept_undo_boundary,
                } => {
                    let operation = client
                        .delete_node(
                            workflow,
                            node,
                            accept_undo_boundary
                                .then_some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
                        )
                        .await?;
                    if args.json {
                        println!("{}", serde_json::to_string(&operation)?);
                    } else {
                        println!(
                            "Node {node} deleted by dependency operation {}; workflow Undo starts at this boundary.",
                            operation.id
                        );
                        println!("{}", operation.detail);
                        for revision in operation.after {
                            println!(
                                "{} {}{}",
                                revision.change_id,
                                revision.commit_id,
                                if revision.conflicted { " CONFLICT" } else { "" }
                            );
                        }
                    }
                }
                NodeCommand::Operations { workflow } => {
                    let operations = client.dependency_operations(workflow).await?;
                    println!(
                        "{}",
                        if args.json {
                            serde_json::to_string(&operations)?
                        } else {
                            serde_json::to_string_pretty(&operations)?
                        }
                    );
                }
                NodeCommand::ReparentRecovery { operation } => {
                    let value = client.inspect_dependency_recovery(operation).await?;
                    println!(
                        "{}",
                        if args.json {
                            serde_json::to_string(&value)?
                        } else {
                            serde_json::to_string_pretty(&value)?
                        }
                    );
                }
                NodeCommand::RecoverReparent {
                    operation,
                    confirm_processes_stopped,
                } => {
                    let value = client
                        .recover_dependency_operation(
                            operation,
                            confirm_processes_stopped
                                .then_some(RecoveryConfirmation::ProcessesStopped),
                        )
                        .await?;
                    println!(
                        "{}",
                        if args.json {
                            serde_json::to_string(&value)?
                        } else {
                            serde_json::to_string_pretty(&value)?
                        }
                    );
                }
                NodeCommand::Review { workflow, node } => {
                    print_review(client.review_node(workflow, node).await?, args.json)?;
                }
                NodeCommand::Refresh { workflow, node } => {
                    let status = client.refresh_node(workflow, node).await?;
                    print_node_change(status, args.json)?;
                }
                NodeCommand::Create {
                    workflow,
                    prompt,
                    from_base,
                    parent,
                } => {
                    let parents = if from_base {
                        vec![NodeParent::WorkflowBase {}]
                    } else {
                        parent
                            .into_iter()
                            .map(|node_id| NodeParent::Node { node_id })
                            .collect()
                    };
                    let node = if parents.is_empty() {
                        client.create_node(workflow, &prompt).await?
                    } else {
                        client
                            .create_node_from_parents(workflow, &prompt, &parents)
                            .await?
                    };
                    print_node(node, args.json)?;
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

fn print_upstream_status(status: &waypoints_core::run::UpstreamStatus) {
    use waypoints_core::run::UpstreamStatus;
    match status {
        UpstreamStatus::Changed => {
            println!("Upstream code or dependencies changed since this run.")
        }
        UpstreamStatus::Unchanged => println!("Upstream revisions unchanged since this run."),
        UpstreamStatus::Unknown { detail } => println!("Upstream comparison unknown: {detail}"),
        _ => println!("Upstream comparison unknown."),
    }
}

fn print_review(value: waypoints_core::jj::RevisionReview, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string(&value)?);
    } else {
        println!("Change: {}", value.revision.change_id);
        println!("Commit: {}", value.revision.commit_id);
        println!("Parents: {}", value.parent_commit_ids.join(" "));
        println!("Conflicted: {}", value.revision.conflicted);
        println!("\n{}", value.revision.description);
        print!("\n{}{}", value.summary, value.patch);
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
