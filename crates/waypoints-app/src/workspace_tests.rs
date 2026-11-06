use super::*;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Bounds, TestAppContext, WindowBounds, WindowOptions, size};

pub(super) fn workflow(project: &str, id: i64) -> Workflow {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "project_path": project,
        "name": format!("Workflow {id}"),
    }))
    .expect("workflow fixture matches the storage schema")
}

pub(super) fn open_workspace(
    cx: &mut TestAppContext,
) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<Workspace>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        init(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(900.), px(700.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let mut workspace = Workspace::new(window, cx);
                    let first = workspace
                        .state
                        .add("project-a".into())
                        .expect("first project");
                    let second = workspace
                        .state
                        .add("project-b".into())
                        .expect("second project");
                    workspace.state.projects[0].workflows =
                        Load::Ready(vec![workflow("project-a", 7)]);
                    workspace.state.projects[1].workflows =
                        Load::Ready(vec![workflow("project-b", 8)]);
                    workspace.state.selected = Some((first, 7));
                    assert_ne!(first, second);
                    workspace
                })
            },
        )
        .expect("open workspace test window")
    })
}

#[gpui_kit::test]
fn sidebar_shortcut_preserves_selection_and_restores_focus(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("workflow-1-7").visible());
        let shortcut = if cfg!(target_os = "macos") {
            "cmd-b"
        } else {
            "ctrl-b"
        };
        window.press(shortcut, cx);
        assert!(window.try_find("workflow-1-7").is_none());
        assert_eq!(workspace.read(cx).state.selected, Some((1, 7)));
        assert!(workspace.read(cx).focus.is_focused(window));
        window.click("toggle-sidebar", cx);
        assert!(window.find("workflow-1-7").visible());
        assert_eq!(workspace.read(cx).state.selected, Some((1, 7)));
    })
    .expect("workspace window remains open");
}

#[gpui_kit::test]
fn oversized_sidebar_keeps_detail_space_after_resize_and_toggle(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update(|cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.sidebar_width = 1000.;
            cx.notify();
        });
    });

    let assert_divider = |window: &mut Window, cx: &mut App| {
        window.render_frame(cx);
        let divider = window.find(("resizable-handle", 0_usize)).bounds();
        let viewport = window.viewport_size();
        assert!(divider.left() >= px(0.), "divider is left of the viewport");
        assert!(
            divider.right() <= viewport.width - px(320.),
            "divider {divider:?} leaves less than 320px in viewport {viewport:?}"
        );
    };

    cx.simulate_window_resize(handle, size(px(720.), px(700.)));
    cx.update_window(handle, |_, window, cx| assert_divider(window, cx))
        .expect("workspace window remains open");
    cx.simulate_window_resize(handle, size(px(1400.), px(700.)));
    cx.update_window(handle, |_, window, cx| assert_divider(window, cx))
        .expect("workspace window remains open");
    cx.simulate_window_resize(handle, size(px(720.), px(700.)));
    cx.update_window(handle, |_, window, cx| {
        assert_divider(window, cx);
        window.click("toggle-sidebar", cx);
        window.click("toggle-sidebar", cx);
        assert_divider(window, cx);
    })
    .expect("workspace window remains open");
}

#[gpui_kit::test]
fn project_expansion_and_removal_keep_navigation_consistent(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("workflow-1-7").visible());
        assert!(window.find("workflow-2-8").visible());
        window.click(("project", 1_u64), cx);
        assert!(window.try_find("workflow-1-7").is_none());
        assert!(window.find("workflow-2-8").visible());
        assert_eq!(workspace.read(cx).state.selected, Some((1, 7)));
        window.click(("project", 1_u64), cx);
        assert!(window.find("workflow-1-7").visible());
        window.click(("project-menu", 2_u64), cx);
        let mut menu = window.within("popup-menu");
        assert_eq!(menu.find(1usize).label(), Some("Remove from Sidebar"));
        menu.click(1usize, cx);
        assert!(window.try_find("workflow-2-8").is_none());
        assert_eq!(workspace.read(cx).state.selected, Some((1, 7)));
        window.click(("project-menu", 1_u64), cx);
        let mut menu = window.within("popup-menu");
        assert_eq!(menu.find(1usize).label(), Some("Remove from Sidebar"));
        menu.click(1usize, cx);
        assert_eq!(workspace.read(cx).state.selected, None);
        assert!(matches!(workspace.read(cx).graph, Load::Idle));
        assert!(workspace.read(cx).focus.is_focused(window));
    })
    .expect("workspace window remains open");
}

#[test]
fn refresh_only_clears_selection_when_its_own_project_loses_the_workflow() {
    let mut state = WorkspaceState::default();
    let first = state.add("project-a".into()).expect("first project");
    let second = state.add("project-b".into()).expect("second project");
    state.selected = Some((first, 7));

    let (_, revision) = state.begin_load(second).expect("second project exists");
    assert!(state.finish_load(second, revision, Ok(vec![])));
    assert_eq!(state.selected, Some((first, 7)));

    let (_, revision) = state.begin_load(first).expect("first project exists");
    assert!(state.finish_load(first, revision, Ok(vec![workflow("project-a", 7)])));
    assert_eq!(state.selected, Some((first, 7)));

    let (_, revision) = state.begin_load(first).expect("first project exists");
    assert!(state.finish_load(first, revision, Ok(vec![])));
    assert_eq!(state.selected, None);
}

#[test]
fn removing_another_project_preserves_selection_and_stale_load_cannot_restore_it() {
    let mut state = WorkspaceState::default();
    let first = state.add("project-a".into()).expect("first project");
    let second = state.add("project-b".into()).expect("second project");
    state.selected = Some((first, 7));
    let (_, revision) = state.begin_load(first).expect("first project exists");

    state.remove(second);
    assert_eq!(state.selected, Some((first, 7)));
    state.remove(first);
    assert_eq!(state.selected, None);
    assert!(!state.finish_load(first, revision, Ok(vec![workflow("project-a", 7)])));
    assert!(state.projects.is_empty());
    assert_eq!(state.selected, None);
}

#[gpui_kit::test]
fn settings_opens_once_and_updates_workspace_appearance(cx: &mut TestAppContext) {
    let (workspace_window, workspace) = open_workspace(cx);
    let settings = cx.update(|cx| {
        crate::settings::register(workspace.downgrade(), cx);
        cx.dispatch_action(&crate::OpenSettings);
        cx.dispatch_action(&crate::OpenSettings);
        let windows = cx.windows();
        assert_eq!(windows.len(), 2, "Settings should reuse its window");
        windows
            .into_iter()
            .find(|handle| *handle != workspace_window)
            .expect("separate settings window")
    });
    cx.update_window(settings, |_, window, cx| {
        window.render_frame(cx);
        let mut choices = window.within("appearance");
        assert_eq!(choices.find(0usize).checked(), Some(true));
        choices.click(2usize, cx);
        assert_eq!(workspace.read(cx).appearance, Appearance::Dark);
        assert!(cx.theme().mode.is_dark());
        let mut choices = window.within("appearance");
        choices.click(1usize, cx);
        assert_eq!(workspace.read(cx).appearance, Appearance::Light);
        assert!(!cx.theme().mode.is_dark());
        window.within("appearance").click(0usize, cx);
        assert_eq!(workspace.read(cx).appearance, Appearance::System);
    })
    .expect("settings window remains open");
}

#[gpui_kit::test]
fn inline_retry_only_reloads_its_project(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update(|cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.state.projects[1].workflows =
                Load::Error(ProjectLoadError::Backend("Backend unavailable".into()));
            cx.notify();
        });
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find(("retry-project", 2_u64)).visible());
        assert!(window.try_find(("retry-project", 1_u64)).is_none());
        window.click(("retry-project", 2_u64), cx);
        let state = &workspace.read(cx).state;
        assert_eq!(state.projects[0].revision, 0);
        assert_eq!(state.projects[1].revision, 1);
        assert!(window.find("workflow-1-7").visible());
        assert_eq!(workspace.read(cx).state.selected, Some((1, 7)));
    })
    .expect("workspace window remains open");
}

#[gpui_kit::test]
fn minimum_window_scrolls_to_later_projects_without_moving_detail(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update(|cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.state.projects[0].workflows =
                Load::Ready((1..=40).map(|id| workflow("project-a", id)).collect());
            cx.notify();
        });
    });
    cx.simulate_window_resize(handle, size(px(720.), px(480.)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let divider = window.find(("resizable-handle", 0_usize)).bounds();
        assert!(!window.find("workflow-2-8").visible());
        window.scroll(
            "workflows",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-1600.))),
            cx,
        );
        assert!(window.find("workflow-2-8").visible());
        assert_eq!(window.find(("resizable-handle", 0_usize)).bounds(), divider);
        assert_eq!(workspace.read(cx).state.selected, Some((1, 7)));
    })
    .expect("workspace window remains open");
}

#[gpui_kit::test]
fn project_right_click_keeps_disclosure_and_menu_closed(cx: &mut TestAppContext) {
    use gpui_kit::{MouseButton, test::TestEventExt as _};

    let (handle, _) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for id in [("project", 1_u64), ("project-menu", 1_u64)] {
            let position = window.find(id).bounds().center();
            window.pointer_down(position, MouseButton::Right, cx);
            assert!(window.find("workflow-1-7").visible());
            assert!(window.try_find("popup-menu").is_none());
            window.pointer_up(position, MouseButton::Right, cx);
            assert!(window.find("workflow-1-7").visible());
            assert!(window.try_find("popup-menu").is_none());
        }
        window.click(("project-menu", 1_u64), cx);
        assert!(window.find("popup-menu").visible());
        assert!(window.find("workflow-1-7").visible());
    })
    .expect("workspace window remains open");
}

#[gpui_kit::test]
async fn project_menu_dismisses_with_escape_or_outside_press_and_restores_focus(
    cx: &mut TestAppContext,
) {
    use std::time::Duration;

    use gpui_kit::MouseButton;
    use gpui_kit::test::{TestAppContextExt as _, TestEventExt as _};

    let (handle, _) = open_workspace(cx);
    let focus_before = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let focus = window.focused(cx);
            window.click(("project-menu", 1_u64), cx);
            assert_eq!(window.find("popup-menu").focused(), Some(true));
            window.within("popup-menu").press("escape", cx);
            focus
        })
        .expect("workspace window remains open");
    cx.wait_for(handle, Duration::from_secs(1), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(
            window.focused(cx),
            focus_before,
            "dismissing the project menu returns focus"
        );
        assert!(window.find("workflow-1-7").visible());

        window.click(("project-menu", 1_u64), cx);
        assert!(window.find("popup-menu").visible());
        let outside = window.find("workflow-1-7").bounds().center();
        window.pointer_down(outside, MouseButton::Left, cx);
        window.pointer_up(outside, MouseButton::Left, cx);
    })
    .expect("workspace window remains open");
    cx.wait_for(handle, Duration::from_secs(1), |window, _| {
        window.try_find("popup-menu").is_none()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("workflow-1-7").visible());
        assert_eq!(window.focused(cx), focus_before);
    })
    .expect("workspace window remains open");
}

#[gpui_kit::test]
fn assignment_preview_preserves_unsaved_source_when_switching_nodes(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.graph = Load::Ready(
                serde_json::from_value(serde_json::json!({
                    "workflow": workflow("project-a", 7),
                    "nodes": [
                        {"id": 1, "workflow_id": 7, "prompt": "# First assignment"},
                        {"id": 2, "workflow_id": 7, "prompt": "# Second assignment"}
                    ],
                    "connections": []
                }))
                .expect("workflow graph fixture matches the storage schema"),
            );
            cx.notify();
        });
        window.render_frame(cx);
        window.click("graph-node-1", cx);
        window.click("input-state", cx);
        window.input("Unsaved text", cx);
        assert!(
            workspace
                .read(cx)
                .assignment_draft((1, 7, 1), cx)
                .contains("Unsaved text")
        );
        window.click("Preview", cx);
        assert!(window.try_find("input-state").is_none());
        window.click("graph-node-2", cx);
        assert_eq!(
            workspace.read(cx).assignment_draft((1, 7, 2), cx).as_ref(),
            "# Second assignment"
        );
        assert!(window.find("input-state").visible());
        window.click("graph-node-1", cx);
        assert!(window.try_find("input-state").is_none());
        window.click("Edit", cx);
        assert!(
            workspace
                .read(cx)
                .assignment_draft((1, 7, 1), cx)
                .contains("Unsaved text")
        );
    })
    .expect("workspace window remains open");
}

#[gpui_kit::test]
fn graph_cards_and_dependency_spacing_scale_with_theme_font(cx: &mut TestAppContext) {
    use gpui_kit::component::Theme;

    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        install_graph_fixture(&workspace, cx);
        window.render_frame(cx);
        let first = window.find("graph-node-1").bounds();
        let second = window.find("graph-node-2").bounds();

        Theme::global_mut(cx).font_size *= 1.5;
        workspace.update(cx, |_, cx| cx.notify());
        window.render_frame(cx);
        let zoomed_first = window.find("graph-node-1").bounds();
        let zoomed_second = window.find("graph-node-2").bounds();
        assert!(zoomed_first.size.height > first.size.height);
        assert!(zoomed_first.size.width > first.size.width);
        assert!(zoomed_second.left() - zoomed_first.right() > second.left() - first.right());
    })
    .expect("workspace window remains open");
}

fn install_graph_fixture(workspace: &gpui_kit::Entity<Workspace>, cx: &mut App) {
    workspace.update(cx, |workspace, cx| {
        workspace.graph = Load::Ready(
            serde_json::from_value(serde_json::json!({
                "workflow": workflow("project-a", 7),
                "nodes": [
                    {"id": 1, "workflow_id": 7, "prompt": "Review the parser"},
                    {"id": 2, "workflow_id": 7, "prompt": "Review the tests"}
                ],
                "connections": [{"workflow_id": 7, "source_id": 1, "target_id": 2}]
            }))
            .expect("workflow graph fixture matches the storage schema"),
        );
        cx.notify();
    });
}

#[gpui_kit::test]
fn short_window_shrinks_the_graph_without_overwriting_its_saved_height(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        install_graph_fixture(&workspace, cx);
        workspace.update(cx, |workspace, cx| workspace.open_node(1, window, cx));
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).graph_height, 192.);
    })
    .expect("workspace window remains open");
    let wide_assignment = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find("Assignment").bounds()
        })
        .expect("workspace window remains open");

    cx.simulate_window_resize(handle, size(px(720.), px(480.)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.find("Assignment").bounds().top() < wide_assignment.top(),
            "short windows give the node workspace priority over the graph"
        );
    })
    .expect("workspace window remains open");

    cx.simulate_window_resize(handle, size(px(900.), px(700.)));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("Assignment").bounds(),
            wide_assignment,
            "growing the window restores the saved graph height"
        );
        assert_eq!(workspace.read(cx).graph_height, 192.);
    })
    .expect("workspace window remains open");
}

#[gpui_kit::test]
fn graph_resize_and_collapse_keep_the_node_workspace_reachable_at_minimum_width(
    cx: &mut TestAppContext,
) {
    let (handle, workspace) = open_workspace(cx);
    cx.simulate_window_resize(handle, size(px(720.), px(520.)));
    cx.update_window(handle, |_, window, cx| {
        install_graph_fixture(&workspace, cx);
        window.render_frame(cx);
        window.click("graph-node-1", cx);
        let assignment = window.find("Assignment").bounds();
        let divider = window
            .within("graph-split")
            .find(("resizable-handle", 0_usize))
            .bounds()
            .center();
        window.drag(divider, divider - gpui_kit::point(px(0.), px(48.)), cx);
        window.render_frame(cx);
        let resized = window.find("Assignment").bounds();
        assert!(
            resized.top() < assignment.top(),
            "graph divider drag gives space to the node workspace"
        );
        assert!(window.find("input-state").visible());

        window.click("toggle-graph", cx);
        assert!(window.try_find("graph-node-1").is_none());
        assert!(window.find("input-state").visible());
        window.click("toggle-graph", cx);
        assert!(window.find("graph-node-1").visible());
        assert_eq!(window.find("Assignment").bounds(), resized);
        assert_eq!(workspace.read(cx).selected_nodes.get(&(1, 7)), Some(&1));
    })
    .expect("workspace window remains open");
}
