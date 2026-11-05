//! Deterministic workspace data for the headless screenshot binary.

use std::path::PathBuf;

use gpui_kit::{Context, Window};
use waypoints_core::storage::Workflow;

use super::node_workspace::ScreenshotSurface;
use super::{Load, Workspace};

fn workflow(project: &str, id: i64, name: &str) -> Workflow {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "project_path": project,
        "name": name,
    }))
    .expect("screenshot workflow fixture matches the storage schema")
}

pub(crate) fn install(
    workspace: &mut Workspace,
    surface: ScreenshotSurface,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let first = workspace
        .state
        .add(PathBuf::from("/tmp/waypoints-screenshots/project-a"))
        .expect("first screenshot project");
    let second = workspace
        .state
        .add(PathBuf::from("/tmp/waypoints-screenshots/project-b"))
        .expect("second screenshot project");
    assert_ne!(first, second);
    workspace.state.projects[0].workflows = Load::Ready(vec![
        workflow("project-a", 7, "Review workspace polish"),
        workflow("project-a", 9, "History pagination"),
    ]);
    workspace.state.projects[1].workflows =
        Load::Ready(vec![workflow("project-b", 8, "Output transcript")]);
    workspace.state.selected = Some((first, 7));
    workspace.sidebar_visible = true;
    workspace.sidebar_width = 244.;
    workspace.graph_collapsed = matches!(
        surface,
        ScreenshotSurface::Output | ScreenshotSurface::History
    );
    workspace.graph_height = 208.;
    workspace.graph = Load::Ready(
        serde_json::from_value(serde_json::json!({
            "workflow": workflow("project-a", 7, "Review workspace polish"),
            "nodes": [
                {"id": 1, "workflow_id": 7, "prompt": "Polish the review workspace and its history."},
                {"id": 2, "workflow_id": 7, "prompt": "Preserve the retained diff state."},
                {"id": 3, "workflow_id": 7, "prompt": "Render structured agent output."},
                {"id": 4, "workflow_id": 7, "prompt": "Verify freshness after a saved snapshot."}
            ],
            "connections": [
                {"workflow_id": 7, "source_id": 1, "target_id": 2},
                {"workflow_id": 7, "source_id": 2, "target_id": 3},
                {"workflow_id": 7, "source_id": 2, "target_id": 4}
            ]
        }))
        .expect("screenshot graph fixture matches the storage schema"),
    );
    workspace.selected_nodes.insert((first, 7), 1);
    workspace.open_node(1, window, cx);
    if let Some(node) = workspace.node_workspaces.get_mut(&(first, 7, 1)) {
        node.install_screenshot_fixture(surface, window, cx);
    }
    cx.notify();
}
