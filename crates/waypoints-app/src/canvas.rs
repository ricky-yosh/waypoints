//! Canvas fixture: a live camera, world-space cards and edges, and a screen-space HUD.

use std::time::Duration;

use rune::tok;
use rune::{
    Color, Cursor, Drag, El, Icon, Live, Phase, PointerButton, Tx, canvas, col, el, row, text,
};
use rune_kit::{IconButton, IconButtonVariant, Kit};

use crate::fixture::{self, CARD_HEIGHT, CARD_WIDTH, Node, WORLD_HEIGHT, WORLD_WIDTH};
use crate::{Mode, State};

pub const MIN_ZOOM: f32 = 0.35;
pub const MAX_ZOOM: f32 = 2.5;
const CAMERA_DEFAULT_PAN: [f32; 2] = [22.0, 44.0];
const CAMERA_DEFAULT_ZOOM: f32 = 0.88;

/// The camera is presentation only. Changing it never touches the fixture graph.
pub struct Camera {
    pub pan: Live<[f32; 2]>,
    pub zoom: Live<f32>,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            pan: Live::new(CAMERA_DEFAULT_PAN),
            zoom: Live::new(CAMERA_DEFAULT_ZOOM),
        }
    }
}

impl Camera {
    pub fn pan(&self) -> [f32; 2] {
        self.pan.get()
    }

    pub fn zoom(&self) -> f32 {
        self.zoom.get()
    }

    pub fn set_pan(&self, pan: [f32; 2]) {
        self.pan.set(pan);
    }

    pub fn reset(&self) {
        self.pan.set(CAMERA_DEFAULT_PAN);
        self.zoom.set(CAMERA_DEFAULT_ZOOM);
    }

    pub fn pan_by(&self, dx: f32, dy: f32) {
        let pan = self.pan();
        self.pan.set([pan[0] + dx, pan[1] + dy]);
    }

    /// Zoom about a window point, keeping the world point under it fixed.
    pub fn zoom_about(&self, screen: [f32; 2], factor: f32) -> bool {
        let old = self.zoom();
        let next = (old * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        if (next - old).abs() < 1e-6 {
            return false;
        }
        let pan = self.pan();
        let ratio = next / old;
        self.pan.set([
            screen[0] - (screen[0] - pan[0]) * ratio,
            screen[1] - (screen[1] - pan[1]) * ratio,
        ]);
        self.zoom.set(next);
        true
    }

    #[cfg(test)]
    pub fn screen_to_world(&self, screen: [f32; 2]) -> [f32; 2] {
        let pan = self.pan();
        let zoom = self.zoom();
        [(screen[0] - pan[0]) / zoom, (screen[1] - pan[1]) / zoom]
    }

    pub fn world_to_screen(&self, world: [f32; 2]) -> [f32; 2] {
        let pan = self.pan();
        let zoom = self.zoom();
        [world[0] * zoom + pan[0], world[1] * zoom + pan[1]]
    }
}

static PLUS: Icon = Icon {
    name: "waypoints-plus",
    view: 16.0,
    body: r#"<path d="M8 3V13M3 8H13" stroke="currentColor" stroke-linecap="round"/>"#,
};
static MINUS: Icon = Icon {
    name: "waypoints-minus",
    view: 16.0,
    body: r#"<path d="M3 8H13" stroke="currentColor" stroke-linecap="round"/>"#,
};
static RESET: Icon = Icon {
    name: "waypoints-reset",
    view: 16.0,
    body: r#"<path d="M3.2 8a4.8 4.8 0 1 0 1.4-3.4M3.2 3.2v3.2h3.2" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#,
};
static PROJECTS: Icon = Icon {
    name: "waypoints-projects",
    view: 16.0,
    body: r#"<rect x="1.5" y="2.5" width="13" height="11" rx="1.5" stroke="currentColor"/><path d="M6 2.5V13.5" stroke="currentColor"/>"#,
};
static SETTINGS: Icon = Icon {
    name: "waypoints-settings",
    view: 16.0,
    body: r#"<path d="M3 4.5H13M3 11.5H13" stroke="currentColor" stroke-linecap="round"/><circle cx="6" cy="4.5" r="1.5" stroke="currentColor"/><circle cx="10" cy="11.5" r="1.5" stroke="currentColor"/>"#,
};

pub fn viewport(s: &State, k: &Kit, dark: bool, size: [f32; 2], insets: [f32; 2]) -> El {
    let theme = tok::theme(dark);
    let grid = k.c.border_base.alpha(if dark { 0.16 } else { 0.22 });
    let edges = k.c.text_faint.alpha(if dark { 0.7 } else { 0.65 });

    el().key("canvas-viewport")
        .w_full()
        .h_full()
        .clip_soft(0.0)
        .fill(theme.bg)
        .on_press(|s: &mut State| {
            s.selected = None;
            s.workbench_open = false;
            s.preview_node = None;
        })
        .on_wheel(wheel)
        .on_magnify(magnify)
        .on_drag(pan_drag)
        .pointer_buttons([PointerButton::Middle])
        .role("group")
        .label("Workflow canvas")
        .child(
            el().key("canvas-world")
                .absolute(0.0, 0.0)
                .size(WORLD_WIDTH, WORLD_HEIGHT)
                .origin(0.0, 0.0)
                .live_offset(&s.camera.pan)
                .live_scale(&s.camera.zoom)
                .child(
                    canvas(fixture::grid_shape(grid))
                        .key("canvas-grid")
                        .absolute(0.0, 0.0)
                        .size(WORLD_WIDTH, WORLD_HEIGHT),
                )
                .child(
                    canvas(fixture::edge_shape(&s.graph, edges))
                        .key("canvas-edges")
                        .absolute(0.0, 0.0)
                        .size(WORLD_WIDTH, WORLD_HEIGHT),
                )
                .children(s.graph.nodes.iter().map(|node| node_card(s, node, k))),
        )
        .children(
            s.preview_node
                .and_then(|id| s.graph.node(id))
                .map(|node| preview(node, s, k, size)),
        )
        .child(hud(s, k, size, insets))
}

fn wheel(s: &mut State, event: rune::WheelEvent, tx: &mut Tx) -> bool {
    let anchored = if event.mods.cmd {
        s.camera
            .zoom_about([event.x, event.y], (-event.dy * 0.0025).exp())
    } else {
        s.camera.pan_by(event.dx, event.dy);
        true
    };
    if anchored {
        tx.live_only();
    }
    true
}

fn magnify(s: &mut State, event: rune::MagnifyEvent, tx: &mut Tx) -> bool {
    if s.camera
        .zoom_about([event.x, event.y], (1.0 + event.delta).clamp(0.1, 4.0))
    {
        tx.live_only();
    }
    true
}

/// Blank-space primary drag, middle drag, and Space + drag all move the camera.
pub fn pan_drag(s: &mut State, drag: &mut Drag, tx: &mut Tx) {
    match drag.phase {
        Phase::Down => s.pan_start = Some(s.camera.pan()),
        Phase::Move | Phase::Up => {
            if let Some(start) = s.pan_start {
                s.camera.set_pan([start[0] + drag.dx, start[1] + drag.dy]);
                tx.live_only();
            }
            if drag.phase == Phase::Up {
                s.pan_start = None;
            }
        }
    }
}

fn select_node(s: &mut State, id: i64) {
    if s.selected != Some(id) {
        s.preview_node = None;
    }
    if s.mode == Mode::Backend {
        s.select_remote_node(id);
    } else {
        s.selected = Some(id);
    }
}

fn open_node(s: &mut State, id: i64) {
    select_node(s, id);
    s.preview_node = None;
    s.workbench_open = true;
}

fn drag_node(s: &mut State, id: i64, drag: &mut Drag, tx: &mut Tx) {
    if s.space_held || tx.button() != PointerButton::Primary {
        pan_drag(s, drag, tx);
        return;
    }
    match drag.phase {
        Phase::Down => {
            let start = s
                .graph
                .node(id)
                .map(|node| [node.x, node.y])
                .unwrap_or([0.0, 0.0]);
            s.selected = Some(id);
            s.node_drag = Some(crate::NodeDrag { node: id, start });
            tx.instant();
        }
        Phase::Move | Phase::Up => {
            if let Some(node_drag) = s.node_drag.filter(|drag| drag.node == id) {
                let zoom = s.camera.zoom();
                let position = if drag.cancelled() {
                    node_drag.start
                } else {
                    [
                        node_drag.start[0] + drag.dx / zoom,
                        node_drag.start[1] + drag.dy / zoom,
                    ]
                };
                if let Some(node) = s.graph.node_mut(id) {
                    node.x = position[0];
                    node.y = position[1];
                }
            }
            if drag.phase == Phase::Up {
                s.node_drag = None;
            }
            tx.instant();
        }
    }
}

fn preview(node: &Node, s: &State, k: &Kit, size: [f32; 2]) -> El {
    let anchor = s.camera.world_to_screen([node.x, node.y]);
    let zoom = s.camera.zoom();
    const WIDTH: f32 = 278.0;
    const HEIGHT: f32 = 240.0;
    const GAP: f32 = 10.0;
    const EDGE: f32 = 18.0;
    let right = anchor[0] + CARD_WIDTH * zoom + GAP;
    let left = anchor[0] - WIDTH - GAP;
    let below = anchor[1] + CARD_HEIGHT * zoom + GAP;
    let above = anchor[1] - HEIGHT - GAP;
    let (x, y) = if right + WIDTH <= size[0] - EDGE {
        (right, anchor[1])
    } else if left >= EDGE {
        (left, anchor[1])
    } else if below + HEIGHT <= size[1] - EDGE {
        (anchor[0], below)
    } else if above >= EDGE {
        (anchor[0], above)
    } else {
        (
            right.clamp(EDGE, (size[0] - WIDTH - EDGE).max(EDGE)),
            anchor[1].clamp(EDGE, (size[1] - HEIGHT - EDGE).max(EDGE)),
        )
    };
    let x = x.clamp(EDGE, (size[0] - WIDTH - EDGE).max(EDGE));
    let y = y.clamp(EDGE, (size[1] - HEIGHT - EDGE).max(EDGE));
    let status = node.status;
    col()
        .key("graph-preview")
        .absolute(x, y)
        .w(WIDTH)
        .max_h(HEIGHT)
        .padding(12.0)
        .gap(7.0)
        .radius(k.radius.lg)
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_base)
        .shadow(4.0, 18.0, Color::BLACK.alpha(0.22))
        .on_press(|_: &mut State| {})
        .on_wheel(|_: &mut State, _, _| true)
        .role("complementary")
        .label(format!("Preview of {}", node.title))
        .child(
            row()
                .w_full()
                .gap(7.0)
                .items_center()
                .child(
                    text(node.title.clone())
                        .font(k.ty.ui_medium)
                        .color(k.c.text_base)
                        .grow(1.0)
                        .wrap()
                        .max_lines(2),
                )
                .child(text(status.label()).font(k.ty.small).color(status.color(k))),
        )
        .child(
            text(if node.summary.is_empty() {
                "No summary recorded.".to_owned()
            } else {
                node.summary.clone()
            })
            .key("graph-preview-summary")
            .font(k.ty.small)
            .color(k.c.text_muted)
            .w_full()
            .wrap()
            .max_lines(6),
        )
        .child(
            text({
                let dependencies = s.graph.incoming(node.id);
                format!(
                    "{dependencies} {} · Escape closes · Return opens",
                    if dependencies == 1 {
                        "dependency"
                    } else {
                        "dependencies"
                    }
                )
            })
            .font(k.ty.small)
            .color(k.c.text_faint)
            .w_full()
            .wrap(),
        )
}

fn node_card(s: &State, node: &Node, k: &Kit) -> El {
    let id = node.id;
    let selected = s.selected == Some(id);
    let status_color = node.status.color(k);
    let title = node.title.clone();
    let label = format!(
        "{}, {}, {} dependencies",
        node.title,
        node.status.label(),
        s.graph.incoming(node.id)
    );
    let border = if selected {
        (2.0, k.c.border_focus)
    } else {
        (1.0, k.c.border_base)
    };
    let fill = if selected { k.c.layer_01 } else { k.c.bg_base };
    let pill = row()
        .padding(3.0)
        .gap(5.0)
        .items_center()
        .radius(999.0)
        .fill(status_color.alpha(0.14))
        .child(
            el().size(5.0, 5.0)
                .radius(2.5)
                .fill(status_color)
                .a11y_hidden(),
        )
        .child(
            text(node.status.label())
                .font(k.ty.small)
                .color(k.c.text_base),
        );

    let summary = (!node.summary.is_empty()).then(|| {
        text(node.summary.clone())
            .font(k.ty.small)
            .color(k.c.text_muted)
            .w_full()
            .wrap()
            .max_lines(2)
    });
    let card = col()
        .key(format!("node-{id}"))
        .absolute(node.x, node.y)
        .size(CARD_WIDTH, CARD_HEIGHT)
        .padding(12.0)
        .gap(7.0)
        .radius(k.radius.lg)
        .fill(fill)
        .border(border.0, border.1)
        .focusable(true)
        .role("button")
        .label(label)
        .cursor(Cursor::Pointer)
        .on_press(move |s: &mut State| select_node(s, id))
        .on_double_press(move |s: &mut State| open_node(s, id))
        .on_drag(move |s: &mut State, drag, tx| drag_node(s, id, drag, tx))
        .hover_delays(Duration::from_millis(260), Duration::from_millis(90))
        .on_hover_change(move |s: &mut State, inside, _| {
            if inside {
                if !s.workbench_open {
                    s.preview_node = Some(id);
                }
            } else if s.preview_node == Some(id) {
                s.preview_node = None;
            }
        })
        .child(
            el().w_full()
                .gap(8.0)
                .items_center()
                .child(
                    el().size(7.0, 7.0)
                        .radius(3.5)
                        .fill(status_color)
                        .a11y_hidden(),
                )
                .child(
                    text(title)
                        .font(k.ty.ui_medium)
                        .color(k.c.text_base)
                        .truncate()
                        .grow(1.0),
                ),
        )
        .children(summary)
        .child(
            row()
                .w_full()
                .items_center()
                .justify_between()
                .child(pill)
                .child(
                    text(format!("{} deps", s.graph.incoming(node.id)))
                        .font(k.ty.small)
                        .color(k.c.text_faint),
                ),
        );

    k.elevate(
        card,
        if selected {
            &k.elevation.floating
        } else {
            &k.elevation.raised
        },
    )
}

fn hud(s: &State, k: &Kit, size: [f32; 2], insets: [f32; 2]) -> El {
    let x = 18.0;
    let y = if insets[0] > 0.0 { insets[1] } else { 18.0 };
    let zoom_in = IconButton::new(&PLUS)
        .variant(IconButtonVariant::Ghost)
        .label("Zoom in")
        .help("Zoom in")
        .on_press(move |s: &mut State| {
            s.camera.zoom_about(camera_center(size), 1.2);
        })
        .build(k)
        .key("zoom-in");
    let zoom_out = IconButton::new(&MINUS)
        .variant(IconButtonVariant::Ghost)
        .label("Zoom out")
        .help("Zoom out")
        .on_press(move |s: &mut State| {
            s.camera.zoom_about(camera_center(size), 1.0 / 1.2);
        })
        .build(k)
        .key("zoom-out");
    let settings = IconButton::new(&SETTINGS)
        .variant(IconButtonVariant::GhostMuted)
        .label("Settings")
        .help("Appearance and motion settings")
        .on_press(|s: &mut State| s.settings_open = true)
        .build(k)
        .key("settings-open");
    let projects = IconButton::new(&PROJECTS)
        .variant(IconButtonVariant::GhostMuted)
        .label(if s.preferences.sidebar_open {
            "Hide projects"
        } else {
            "Show projects"
        })
        .help("Show or hide the project navigator")
        .on_press(|s: &mut State| s.toggle_sidebar())
        .build(k)
        .key("project-toggle");
    let reset = IconButton::new(&RESET)
        .variant(IconButtonVariant::GhostMuted)
        .label("Reset")
        .help("Reset the canvas view")
        .on_press(|s: &mut State| {
            s.camera.reset();
        })
        .build(k)
        .key("zoom-reset");
    let new_workflow = IconButton::new(&PLUS)
        .variant(IconButtonVariant::GhostMuted)
        .label("New workflow")
        .help("Show the harness prompt for creating a workflow")
        .on_press(|s: &mut State| s.open_workflow_help("new-workflow-hud"))
        .build(k)
        .key("new-workflow-hud");

    let mut controls = row()
        .key("canvas-hud")
        .absolute(x, y)
        .padding(10.0)
        .gap(8.0)
        .items_center()
        .radius(k.radius.lg)
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_base)
        .shadow(1.0, 3.0, Color::BLACK.alpha(0.12))
        .child(
            el().gap(7.0)
                .items_center()
                .child(
                    el().size(8.0, 8.0)
                        .radius(4.0)
                        .fill(k.c.bg_accent)
                        .a11y_hidden(),
                )
                .child(
                    text(if s.mode == Mode::Backend {
                        "Waypoints · Rune canvas"
                    } else {
                        "Waypoints · Rune canvas fixture"
                    })
                    .font(k.ty.ui_medium)
                    .color(k.c.text_base),
                ),
        )
        .child(zoom_out)
        .child(zoom_in)
        .child(reset);
    if s.mode == Mode::Backend && !s.display_fixture {
        controls = controls.child(projects);
        if s.project_path.is_some() {
            controls = controls.child(new_workflow);
        }
    }
    controls = controls.child(settings);
    controls = if let Some(error) = &s.sync_error {
        // Background refresh failures must stay visible when the project
        // navigator is hidden.
        controls.child(
            text("Reconnecting")
                .key("canvas-sync-status")
                .font(k.ty.small)
                .color(k.c.warning.fg)
                .help(error.clone())
                .role("status"),
        )
    } else {
        controls.child(
            text(format!(
                "{} nodes · {} edges",
                s.graph.nodes.len(),
                s.graph.edges.len()
            ))
            .key("canvas-node-count")
            .font(k.ty.small)
            .color(k.c.text_muted),
        )
    };

    let hint =
        text("Drag to pan · pinch or ⌘scroll to zoom · hover or ⇧Return to preview · double-click or Return to open")
            .key("canvas-hint")
            .font(k.ty.small)
            .color(k.c.text_faint)
            .absolute(x, y + 44.0);

    el().child(controls).child(hint)
}

pub fn camera_center(size: [f32; 2]) -> [f32; 2] {
    [size[0] * 0.5, size[1] * 0.5]
}
