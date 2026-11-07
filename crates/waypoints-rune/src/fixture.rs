//! Deterministic fixture data for the Rune canvas. No backend or project access.

use std::collections::{BTreeMap, VecDeque};

use rune::{Color, LineCap, Path, Shape};
use rune_kit::Kit;
use waypoints_core::storage::WorkflowGraph;

pub const CARD_WIDTH: f32 = 216.0;
pub const CARD_HEIGHT: f32 = 112.0;
pub const WORLD_WIDTH: f32 = 2400.0;
pub const WORLD_HEIGHT: f32 = 1400.0;
pub const GRID_STEP: f32 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Done,
    Running,
    NeedsPermission,
    Blocked,
    Ready,
    Unknown,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Done => "Done",
            Status::Running => "Running",
            Status::NeedsPermission => "Needs permission",
            Status::Blocked => "Blocked",
            Status::Ready => "Ready",
            Status::Unknown => "Unknown",
        }
    }

    pub fn color(self, k: &Kit) -> Color {
        match self {
            Status::Done => k.c.success.fg,
            Status::Running => k.c.info.fg,
            Status::NeedsPermission => k.c.warning.fg,
            Status::Blocked => k.c.danger.fg,
            Status::Ready => k.c.text_muted,
            Status::Unknown => k.c.text_faint,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: i64,
    pub title: String,
    pub assignment: String,
    pub summary: String,
    pub status: Status,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub source: i64,
    pub target: i64,
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl Graph {
    pub fn node(&self, id: i64) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn node_mut(&mut self, id: i64) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }

    pub fn incoming(&self, id: i64) -> usize {
        self.edges.iter().filter(|edge| edge.target == id).count()
    }

    pub fn edge_endpoints(&self, edge: Edge) -> Option<([f32; 2], [f32; 2])> {
        let source = self.node(edge.source)?;
        let target = self.node(edge.target)?;
        Some((
            [source.x + CARD_WIDTH, source.y + CARD_HEIGHT * 0.5],
            [target.x, target.y + CARD_HEIGHT * 0.5],
        ))
    }
}

fn node(id: i64, title: &str, summary: &str, status: Status, x: f32, y: f32) -> Node {
    Node {
        id,
        title: title.to_owned(),
        assignment: format!(
            "# {title}\n\n{summary}\n\nFixture assignment for node {id}. Dragging changes presentation only; the backend is not connected."
        ),
        summary: summary.to_owned(),
        status,
        x,
        y,
    }
}

/// The default nine-node fixture used by the canvas and workbench scenes.
pub fn small_graph() -> Graph {
    let nodes = vec![
        node(
            1,
            "Intake",
            "Capture the request and constraints",
            Status::Done,
            72.0,
            152.0,
        ),
        node(
            2,
            "Research APIs",
            "Inspect the pinned framework revision",
            Status::Done,
            72.0,
            352.0,
        ),
        node(
            3,
            "Plan migration",
            "Split work into reversible milestones",
            Status::Running,
            352.0,
            244.0,
        ),
        node(
            4,
            "Implement canvas",
            "Camera, selection and drag interactions",
            Status::Ready,
            632.0,
            128.0,
        ),
        node(
            5,
            "Write fixture notes",
            "Record evidence and remaining native checks",
            Status::Ready,
            632.0,
            352.0,
        ),
        node(
            6,
            "Verify interactions",
            "Script pan, zoom, drag and selection",
            Status::Blocked,
            912.0,
            112.0,
        ),
        node(
            7,
            "Fix regressions",
            "Keep the GPUI frontend working",
            Status::Ready,
            912.0,
            312.0,
        ),
        node(
            8,
            "Review diff",
            "Compare behavior, not layout code",
            Status::Unknown,
            1192.0,
            212.0,
        ),
        node(
            9,
            "Prepare cutover",
            "Wait for explicit approval",
            Status::Ready,
            1192.0,
            408.0,
        ),
    ];
    let edges = vec![
        Edge {
            source: 1,
            target: 3,
        },
        Edge {
            source: 2,
            target: 3,
        },
        Edge {
            source: 3,
            target: 4,
        },
        Edge {
            source: 3,
            target: 5,
        },
        Edge {
            source: 4,
            target: 6,
        },
        Edge {
            source: 6,
            target: 7,
        },
        Edge {
            source: 4,
            target: 8,
        },
        Edge {
            source: 5,
            target: 8,
        },
        Edge {
            source: 7,
            target: 8,
        },
        Edge {
            source: 8,
            target: 9,
        },
    ];
    Graph { nodes, edges }
}

/// A wider fixture graph: 24 cards in six columns with four rows of edges.
pub fn large_graph() -> Graph {
    const TITLES: [&str; 12] = [
        "Intake and scope",
        "Inspect framework",
        "Map reference UI",
        "Draft fixture state",
        "Sketch camera math",
        "Wire live values",
        "Hit-test selection",
        "Drag presentation",
        "Draw edges",
        "Floating workbench",
        "Script interactions",
        "Review evidence",
    ];
    let statuses = [
        Status::Done,
        Status::Done,
        Status::Running,
        Status::Ready,
        Status::Blocked,
        Status::Unknown,
        Status::Ready,
        Status::Done,
    ];
    let mut nodes = Vec::with_capacity(24);
    for column in 0..6_i64 {
        for row in 0..4_i64 {
            let id = column * 4 + row + 1;
            let title = TITLES[(id as usize - 1) % TITLES.len()];
            let status = statuses[(id as usize - 1) % statuses.len()];
            nodes.push(node(
                id,
                &format!("{title} {}", id),
                "Large-graph fixture card for camera and streaming checks",
                status,
                64.0 + column as f32 * 268.0,
                92.0 + row as f32 * 164.0,
            ));
        }
    }
    let mut edges = Vec::new();
    for column in 0..5_i64 {
        for row in 0..4_i64 {
            let source = column * 4 + row + 1;
            edges.push(Edge {
                source,
                target: source + 4,
            });
            if row < 3 {
                edges.push(Edge {
                    source,
                    target: source + 5,
                });
            }
        }
    }
    Graph { nodes, edges }
}

/// Grid lines in world coordinates, drawn inside the scaled world.
pub fn grid_shape(color: Color) -> Shape {
    let mut shape = Shape::new();
    let mut x = 0.0;
    while x <= WORLD_WIDTH {
        shape = shape.line([x, 0.0], [x, WORLD_HEIGHT], 1.0, LineCap::Butt, color);
        x += GRID_STEP;
    }
    let mut y = 0.0;
    while y <= WORLD_HEIGHT {
        shape = shape.line([0.0, y], [WORLD_WIDTH, y], 1.0, LineCap::Butt, color);
        y += GRID_STEP;
    }
    shape
}

/// Connected edges and arrowheads in world coordinates.
pub fn edge_shape(graph: &Graph, color: Color) -> Shape {
    let mut shape = Shape::new();
    for edge in &graph.edges {
        let Some((start, end)) = graph.edge_endpoints(*edge) else {
            continue;
        };
        let middle = start[0] + (end[0] - start[0]) * 0.5;
        let path = Path::new()
            .move_to(start)
            .cubic_to([middle, start[1]], [middle, end[1]], end);
        shape = shape.stroke_path(&path, 1.5, LineCap::Round, color);
        shape = shape.stroke(
            vec![
                [end[0] - 8.0, end[1] - 4.5],
                end,
                [end[0] - 8.0, end[1] + 4.5],
            ],
            1.5,
            color,
        );
    }
    shape
}

/// Converts a backend workflow graph into canvas presentation data. The graph's
/// saved ids, prompts and connections are preserved; only card positions are
/// derived for display.
pub fn from_workflow(workflow: &WorkflowGraph) -> Graph {
    let column_step = CARD_WIDTH + 64.0;
    let row_step = CARD_HEIGHT + 38.0;
    let mut incoming: BTreeMap<i64, usize> =
        workflow.nodes.iter().map(|node| (node.id, 0)).collect();
    let mut outgoing = BTreeMap::<i64, Vec<i64>>::new();
    for edge in &workflow.connections {
        if let Some(count) = incoming.get_mut(&edge.target_id) {
            *count += 1;
            outgoing
                .entry(edge.source_id)
                .or_default()
                .push(edge.target_id);
        }
    }
    let mut ready: VecDeque<i64> = incoming
        .iter()
        .filter_map(|(&id, &count)| (count == 0).then_some(id))
        .collect();
    let mut depths = BTreeMap::<i64, usize>::new();
    while let Some(source) = ready.pop_front() {
        let next_depth = depths.get(&source).copied().unwrap_or_default() + 1;
        for target in outgoing.get(&source).into_iter().flatten() {
            let depth = depths.entry(*target).or_default();
            *depth = (*depth).max(next_depth);
            if let Some(count) = incoming.get_mut(target) {
                *count -= 1;
                if *count == 0 {
                    ready.push_back(*target);
                }
            }
        }
    }
    let mut counts = BTreeMap::<usize, usize>::new();
    for node in &workflow.nodes {
        *counts
            .entry(depths.get(&node.id).copied().unwrap_or_default())
            .or_default() += 1;
    }
    let max_rows = counts.values().copied().max().unwrap_or_default();
    let mut rows = BTreeMap::<usize, usize>::new();
    let nodes = workflow
        .nodes
        .iter()
        .map(|core| {
            let depth = depths.get(&core.id).copied().unwrap_or_default();
            let row = rows.entry(depth).or_default();
            let (title, summary) = prompt_text(&core.prompt);
            let position = (
                72.0 + depth as f32 * column_step,
                72.0 + (*row as f32 + (max_rows - counts[&depth]) as f32 * 0.5) * row_step,
            );
            *row += 1;
            Node {
                id: core.id,
                title,
                assignment: core.prompt.clone(),
                summary,
                status: Status::Unknown,
                x: position.0,
                y: position.1,
            }
        })
        .collect();
    let edges = workflow
        .connections
        .iter()
        .map(|edge| Edge {
            source: edge.source_id,
            target: edge.target_id,
        })
        .collect();
    Graph { nodes, edges }
}

/// The first non-empty prompt line and the next line, matching the graph's card copy.
pub fn prompt_text(prompt: &str) -> (String, String) {
    let mut lines = prompt
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let title = lines
        .next()
        .map(str::to_owned)
        .unwrap_or_else(|| "Untitled node".to_owned());
    let summary = lines.next().map(str::to_owned).unwrap_or_default();
    (title, summary)
}

const SYNTHETIC_OUTPUT: [&str; 8] = [
    "scan · fixture graph mounted",
    "camera · pointer-anchored zoom settled",
    "selection · card identity retained",
    "drag · presentation position updated",
    "edges · paths rebuilt from fixture ids",
    "workbench · screen-space panel focused",
    "output · synthetic tool update received",
    "check · no backend request issued",
];

/// One deterministic line for the fixture workbench's synthetic stream.
pub fn synthetic_line(seq: u64) -> String {
    SYNTHETIC_OUTPUT[seq as usize % SYNTHETIC_OUTPUT.len()].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_edges_reference_existing_unique_nodes() {
        for graph in [small_graph(), large_graph()] {
            let mut ids: Vec<_> = graph.nodes.iter().map(|node| node.id).collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), graph.nodes.len(), "node identities are unique");
            for edge in &graph.edges {
                assert!(
                    graph.node(edge.source).is_some(),
                    "source {} exists",
                    edge.source
                );
                assert!(
                    graph.node(edge.target).is_some(),
                    "target {} exists",
                    edge.target
                );
                assert!(graph.edge_endpoints(*edge).is_some());
            }
        }
    }

    #[test]
    fn edge_endpoints_follow_presentation_position() {
        let mut graph = small_graph();
        let edge = graph.edges[0];
        let (_, before) = graph.edge_endpoints(edge).unwrap();
        let node = graph.node_mut(edge.target).unwrap();
        node.x += 40.0;
        node.y -= 12.0;
        let (_, after) = graph.edge_endpoints(edge).unwrap();
        assert_eq!(after, [before[0] + 40.0, before[1] - 12.0]);
    }
}
