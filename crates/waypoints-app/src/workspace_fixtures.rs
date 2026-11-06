//! Deterministic workspace data for the headless screenshot binary.

use std::path::PathBuf;

use gpui_kit::{Context, Window};
use serde_json::json;
use waypoints_core::{run::RunState, storage::Workflow};

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
    workspace.fixture_mode = true;
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
    workspace.sidebar_width = 210.;
    let overview = matches!(
        surface,
        ScreenshotSurface::Overview | ScreenshotSurface::CompactOverview
    );
    let mut nodes = vec![
        (1, "Review workspace"),
        (2, "Review diffs\nKeep each file’s review context."),
        (3, "Render output\nKeep tool and assistant output separate."),
        (4, "Integration tests\nCheck the editing and run workflows."),
    ];
    let mut edges = vec![(1, 2), (1, 3), (2, 4), (3, 4)];
    if overview {
        nodes.extend([
            (5, "Fix stale review\nKeep pinned revisions stable."),
            (6, "Record revision\nSave the native change."),
            (7, "Publish build\nCheck the package permissions."),
        ]);
        edges.extend([(4, 5), (4, 6), (5, 7), (6, 7)]);
    } else {
        nodes[0].1 = "Polish the node workspace.";
    }
    workspace.graph = Load::Ready(
        serde_json::from_value(json!({
            "workflow": workflow("project-a", 7, "Review workspace polish"),
            "nodes": nodes.iter().map(|(id, prompt)| json!({"id": id, "workflow_id": 7, "prompt": prompt})).collect::<Vec<_>>(),
            "connections": edges.iter().map(|(source, target)| json!({"workflow_id": 7, "source_id": source, "target_id": target})).collect::<Vec<_>>()
        }))
        .expect("screenshot graph fixture matches the storage schema"),
    );
    if overview {
        for id in 1..=7 {
            workspace.open_node(id, window, cx);
            if let Some(node) = workspace.node_workspaces.get_mut(&(first, 7, id)) {
                match id {
                    2 => node.install_screenshot_activity(id, RunState::Running, cx),
                    4 => node.install_qa_permission(id, 100 + id, "Run integration tests"),
                    5 => node.install_screenshot_activity(id, RunState::Completed, cx),
                    6 => node.install_screenshot_activity(id, RunState::Finalizing, cx),
                    7 => node.install_screenshot_activity(
                        id,
                        RunState::Failed {
                            detail: "Package registry refused the request.".into(),
                        },
                        cx,
                    ),
                    _ => node.set_screenshot_unrun(),
                }
            }
        }
        workspace.open_workbenches.clear();
        workspace.selected_nodes.insert((first, 7), 3);
        if matches!(surface, ScreenshotSurface::CompactOverview) {
            workspace.graph_cameras.entry((first, 7)).or_default();
            workspace.set_graph_zoom((first, 7), 0, cx);
        } else {
            workspace.graph_preview = Some(2);
        }
        cx.notify();
        return;
    }
    workspace.selected_nodes.insert((first, 7), 1);
    workspace.open_node(1, window, cx);
    if let Some(node) = workspace.node_workspaces.get_mut(&(first, 7, 1)) {
        node.install_screenshot_fixture(surface, window, cx);
        if matches!(surface, ScreenshotSurface::Output) {
            node.install_screenshot_activity(1, RunState::Running, cx);
        }
    }
    cx.notify();
}
