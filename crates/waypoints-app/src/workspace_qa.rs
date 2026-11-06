//! Native window QA harness with deterministic fixture data.
//!
//! This is intentionally not a real backend or agent session. It drives the
//! same GPUI window, view tree, retained state, menus, dialogs, and event
//! dispatch as the desktop app so the minimum-size, zoomed, and freshness
//! interactions can be checked without macOS Accessibility or Screen
//! Recording permission.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::Theme;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    App, AppContext as _, Bounds, Context, Entity, ScrollDelta, Window, WindowBounds,
    WindowOptions, point, px, size,
};
use waypoints_core::{run::RunState, storage::Workflow};

use super::{Load, ScreenshotSurface, Workspace};

pub(crate) fn run() {
    let result = Arc::new(Mutex::new(None));
    let outcome = result.clone();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            super::init(cx);
            // Exercise the documented 1.5x interface scale at the minimum
            // outer window size.
            Theme::global_mut(cx).font_size *= 1.5;
            let (handle, view) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(40.), px(40.)),
                        size(px(720.), px(480.)),
                    ))),
                    window_min_size: Some(size(px(720.), px(480.))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        let mut workspace = Workspace::new(window, cx);
                        install_fixture(&mut workspace, window, cx);
                        workspace
                    })
                },
            )
            .expect("open native QA window");
            cx.activate(true);

            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                let result = handle
                    .update(cx, |_, window, cx| drive(&view, window, cx))
                    .unwrap_or_else(|error| Err(format!("window update failed: {error:#}")));
                match &result {
                    Ok(()) => eprintln!("NATIVE_QA_OK"),
                    Err(error) => eprintln!("NATIVE_QA_FAIL: {error}"),
                }
                *outcome.lock().expect("QA result lock") = Some(result);
                cx.update(|cx| cx.quit());
            })
            .detach();
        });

    match result.lock().expect("QA result lock").take() {
        Some(Ok(())) => eprintln!("NATIVE_QA_OK"),
        Some(Err(error)) => {
            eprintln!("NATIVE_QA_FAIL: {error}");
            std::process::exit(1);
        }
        None => {
            eprintln!("NATIVE_QA_FAIL: scenario did not run");
            std::process::exit(2);
        }
    }
}

fn install_fixture(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    workspace.fixture_mode = true;
    let first = workspace
        .state
        .add(PathBuf::from("/tmp/waypoints-native-qa/project-a"))
        .expect("first QA project");
    workspace.state.projects[0].workflows =
        Load::Ready(vec![workflow("project-a", 7, "Native QA workflow")]);
    workspace.state.selected = Some((first, 7));
    workspace.graph = Load::Ready(
        serde_json::from_value(serde_json::json!({
            "workflow": workflow("project-a", 7, "Native QA workflow"),
            "nodes": (1..=6).map(|id| serde_json::json!({
                "id": id,
                "workflow_id": 7,
                "prompt": format!("QA node {id}")
            })).collect::<Vec<_>>(),
            "connections": [
                {"workflow_id": 7, "source_id": 1, "target_id": 2},
                {"workflow_id": 7, "source_id": 2, "target_id": 3},
                {"workflow_id": 7, "source_id": 3, "target_id": 4}
            ]
        }))
        .expect("native QA graph fixture matches the storage schema"),
    );
    workspace.selected_nodes.insert((first, 7), 1);
    // Start expanded to exercise the editor at minimum window size.
    workspace.expanded_workbench = true;
    workspace.open_node(1, window, cx);

    if let Some(node) = workspace.node_workspaces.get_mut(&(first, 7, 1)) {
        node.install_screenshot_fixture(ScreenshotSurface::Output, window, cx);
        node.install_screenshot_fixture(ScreenshotSurface::Changes, window, cx);
        node.install_screenshot_fixture(ScreenshotSurface::History, window, cx);
        node.install_screenshot_fixture(ScreenshotSurface::Assignment, window, cx);
        node.set_qa_run_idle();
        node.set_qa_review_stale(false);
        node.set_qa_history_stale(false);
    }

    for id in 2..=6 {
        workspace.open_node(id, window, cx);
    }
    workspace.open_node(1, window, cx);
    cx.notify();
}

fn workflow(project: &str, id: i64, name: &str) -> Workflow {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "project_path": project,
        "name": name,
    }))
    .expect("native QA workflow fixture matches the storage schema")
}

fn drive(workspace: &Entity<Workspace>, window: &mut Window, cx: &mut App) -> Result<(), String> {
    window.render_frame(cx);
    println!(
        "NATIVE_QA viewport {}x{}",
        f32::from(window.viewport_size().width),
        f32::from(window.viewport_size().height)
    );
    require(window, "workflow-1-7", "selected workflow")?;

    // Restore the floating surface, close it with the keyboard, and reopen
    // the same assignment from its restored graph focus.
    window.click("expand-workbench", cx);
    window.render_frame(cx);
    require(
        window,
        "workbench-resize",
        "floating workbench resize handle",
    )?;
    require(window, "graph-node-1", "workflow graph node")?;
    let graph = window.find("graph-node-1").bounds();
    let floating = window.find("workbench").bounds();
    let grip = window.find("workbench-resize").bounds().center();
    window.drag(grip, grip - point(px(0.), px(18.)), cx);
    window.render_frame(cx);
    check(
        window.find("workbench").bounds().top() < floating.top(),
        "workbench did not resize",
    )?;
    check(
        window.find("graph-node-1").bounds() == graph,
        "resizing moved the graph",
    )?;
    window.press("escape", cx);
    window.render_frame(cx);
    check(
        window.try_find("workbench").is_none(),
        "workbench stayed open",
    )?;
    window.press("shift-enter", cx);
    window.render_frame(cx);
    require(
        window,
        "open-preview-workbench",
        "anchored keyboard preview",
    )?;
    window.press("escape", cx);
    window.render_frame(cx);
    check(
        window.try_find("open-preview-workbench").is_none(),
        "graph preview stayed open",
    )?;
    window.click("graph-zoom-out", cx);
    window.click("graph-zoom-out", cx);
    window.render_frame(cx);
    let compact = window.find("graph-node-1").bounds().size.width;
    window.click("graph-zoom-reset", cx);
    window.render_frame(cx);
    check(
        window.find("graph-node-1").bounds().size.width > compact,
        "zoom reset did not restore the graph size",
    )?;
    window.click("graph-node-1", cx);
    window.render_frame(cx);
    require(window, "close-workbench", "reopened workbench")?;
    window.click("expand-workbench", cx);
    window.render_frame(cx);
    eprintln!("NATIVE_QA_OK floating workbench, Escape and keyboard reopen");

    // Assignment: type a local draft, then let a project refresh replace the
    // saved assignment. The dirty draft must be preserved with a warning.
    window.click("Edit", cx);
    window.render_frame(cx);
    window.scroll(
        "assignment-content",
        ScrollDelta::Pixels(point(px(0.), px(-400.))),
        cx,
    );
    window.render_frame(cx);
    window.click("input-state", cx);
    workspace.update(cx, |workspace, cx| {
        workspace
            .node_workspaces
            .get(&(1, 7, 1))
            .expect("QA assignment node")
            .focus_qa_assignment(window, cx);
    });
    window.input(" Local QA edit", cx);
    let draft = workspace.read(cx).assignment_draft((1, 7, 1), cx);
    check(
        draft.contains("Local QA edit"),
        "assignment edit was not retained",
    )?;

    workspace.update(cx, |workspace, cx| {
        workspace.graph = Load::Ready(
            serde_json::from_value(serde_json::json!({
                "workflow": workflow("project-a", 7, "Native QA workflow"),
                "nodes": (1..=6).map(|id| serde_json::json!({
                    "id": id,
                    "workflow_id": 7,
                    "prompt": if id == 1 { "Externally saved QA assignment" } else { "QA node" }
                })).collect::<Vec<_>>(),
                "connections": []
            }))
            .expect("native QA refresh fixture matches the storage schema"),
        );
        cx.notify();
    });
    window.render_frame(cx);
    require(
        window,
        "load-external-assignment",
        "external assignment warning",
    )?;
    window.click("load-external-assignment", cx);
    let draft = workspace.read(cx).assignment_draft((1, 7, 1), cx);
    check(
        draft == "Externally saved QA assignment",
        "Load saved assignment did not adopt the refreshed text",
    )?;
    eprintln!("NATIVE_QA_OK assignment refresh");

    window.click("continue-session", cx);
    require(window, "submit-follow-up", "assignment follow-up dialog")?;
    window.press("escape", cx);
    check(
        wait_for_absent(window, "submit-follow-up", cx),
        "assignment follow-up did not dismiss",
    )?;

    // Multiple pending permissions stay visible as separate, compact
    // controls and remain answerable over the current workspace.
    workspace.update(cx, |workspace, cx| {
        for id in 2..=6 {
            if let Some(node) = workspace.node_workspaces.get_mut(&(1, 7, id)) {
                node.install_qa_permission(id, 100 + id, &format!("Write QA fixture {id}"));
            }
        }
        cx.notify();
    });
    window.render_frame(cx);
    for id in 2..=6 {
        require(
            window,
            format!("permission-{}", 100 + id),
            "pending permission",
        )?;
    }
    window.click("permission-102", cx);
    require(window, "dismiss-permission", "permission dialog")?;
    window.press("escape", cx);
    check(
        wait_for_absent(window, "dismiss-permission", cx),
        "permission dialog did not dismiss",
    )?;
    workspace.update(cx, |workspace, cx| {
        for id in 2..=6 {
            if let Some(node) = workspace.node_workspaces.get_mut(&(1, 7, id)) {
                node.clear_qa_permission();
            }
        }
        cx.notify();
    });
    window.render_frame(cx);
    eprintln!("NATIVE_QA_OK permissions");

    // History: a fresh run is announced without reloading the list.
    window.click("History", cx);
    workspace.update(cx, |workspace, cx| {
        let node = workspace
            .node_workspaces
            .get_mut(&(1, 7, 1))
            .expect("QA history node");
        node.set_qa_history_stale(true);
        cx.notify();
    });
    window.render_frame(cx);
    require(window, "history-stale", "history freshness notice")?;
    for run in [42, 41] {
        let id = format!("history-run-{run}");
        let navigation = window.find("history-navigation").bounds();
        if window
            .try_find(id.clone())
            .is_none_or(|row| !navigation.contains(&row.bounds().center()))
        {
            window.scroll(
                "history-navigation",
                ScrollDelta::Pixels(point(px(0.), px(-65.))),
                cx,
            );
            window.render_frame(cx);
        }
        window.click(id, cx);
        window.render_frame(cx);
        if run == 42 {
            require(window, "run-42", "review selected run snapshot")?;
        } else {
            check(
                window.try_find("run-42").is_none(),
                "the previous run review command remained visible",
            )?;
        }
    }
    eprintln!("NATIVE_QA_OK history freshness");

    // Changes: review target, diff controls, and stale-current notice.
    window.click("Changes", cx);
    window.render_frame(cx);
    require(window, "wrap-review", "diff wrap control")?;
    require(window, "split-review", "diff split control")?;
    workspace.update(cx, |workspace, cx| {
        let node = workspace
            .node_workspaces
            .get_mut(&(1, 7, 1))
            .expect("QA review node");
        node.set_qa_review_stale(true);
        cx.notify();
    });
    window.render_frame(cx);
    require(window, "review-stale", "stale review notice")?;
    eprintln!("NATIVE_QA_OK review");

    // Output: at 1.5x text in the minimum window, the follow-up trigger moves
    // beside the node title and opens a dialog instead of falling below the
    // window.
    workspace.update(cx, |workspace, cx| {
        workspace.expanded_workbench = false;
        cx.notify();
    });
    window.render_frame(cx);
    require(window, "graph-node-1", "workflow graph node")?;
    window.click("Output", cx);
    window.render_frame(cx);
    require(window, "continue-session", "compact continue trigger")?;
    window.click("continue-session", cx);
    require(window, "submit-follow-up", "follow-up dialog")?;
    require(window, "cancel-follow-up", "follow-up dialog cancel")?;
    window.press("escape", cx);
    check(
        wait_for_absent(window, "submit-follow-up", cx),
        "follow-up dialog did not dismiss",
    )?;
    eprintln!("NATIVE_QA_OK run follow-up at minimum size and 1.5x text");

    workspace.update(cx, |workspace, cx| {
        if let Some(node) = workspace.node_workspaces.get_mut(&(1, 7, 1)) {
            node.install_screenshot_activity(1, RunState::Running, cx);
        }
        cx.notify();
    });
    window.render_frame(cx);
    require(window, "cancel-run", "running toolbar cancel control")?;
    window.click("close-workbench", cx);
    window.render_frame(cx);
    check(
        workspace
            .read(cx)
            .graph_activity((1, 7, 1))
            .run
            .is_some_and(|run| run.state == RunState::Running),
        "closing the workbench changed the run state",
    )?;
    eprintln!("NATIVE_QA_OK running toolbar and close without cancellation");

    Ok(())
}

fn require(window: &Window, id: impl Into<gpui_kit::ElementId>, label: &str) -> Result<(), String> {
    let id = id.into();
    window
        .try_find(id.clone())
        .filter(|element| element.visible())
        .map(|_| ())
        .ok_or_else(|| format!("missing visible {label} ({id:?})"))
}

fn wait_for_absent(window: &mut Window, id: impl Into<gpui_kit::ElementId>, cx: &mut App) -> bool {
    let id = id.into();
    for _ in 0..20 {
        if window
            .try_find(id.clone())
            .is_none_or(|element| !element.visible())
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
        window.render_frame(cx);
    }
    window.try_find(id).is_none_or(|element| !element.visible())
}

fn check(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned())
    }
}
