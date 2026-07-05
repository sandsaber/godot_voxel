//! Graph topology + AST-walker interpreter for the procedural-graph runtime.
//!
//! A [`Graph`] owns its nodes; each node has a [`NodeKind`] describing what
//! it computes, plus input ports referencing upstream nodes by id. The
//! interpreter walks the graph in topological order, evaluating every node
//! over a Y-slice of voxels at a time and storing the resulting f32 buffer
//! back on the node for downstream consumption.
//!
//! This module is intentionally engine-agnostic and `VoxelBuffer`-free; the
//! [`super::generator_graph::GraphGenerator`] adapter wires it into the
//! [`crate::generators::base::VoxelGenerator`] trait.

/// Identifies a node inside a [`Graph`]. The caller picks ids; they need not
/// be dense or contiguous, but must be unique within a graph.
pub type GraphNodeId = u32;

/// A typed port on a node. `node` is the upstream producer; `port` selects
/// which of that producer's outputs to read (port 0 for single-output nodes,
/// the only case this minimal port supports).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphPort {
    pub node: GraphNodeId,
}

impl GraphPort {
    pub fn new(node: GraphNodeId) -> Self {
        Self { node }
    }
}

/// Optional parameter value attached to a node (e.g. the constant value of a
/// `Constant` node, the remap range of a `Remap` node).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GraphParam {
    /// Single f32 constant.
    F(f32),
    /// Two-component range (used by Remap: `(from_start, from_end)`).
    RangeFrom(f32, f32),
    /// Two-component range (used by Remap: `(to_start, to_end)`).
    RangeTo(f32, f32),
}

/// What a node computes. Variants bundle parameters and a fixed-size list of
/// input ports (matching the C++ `NodeType` port counts). Inputs that aren't
/// connected default to `0.0` at execution time.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    /// World X coordinate of the voxel (block-relative, scaled by LOD).
    InputX,
    /// World Y coordinate.
    InputY,
    /// World Z coordinate.
    InputZ,
    /// Constant value. Carries the value as a parameter.
    Constant(f32),
    /// `a + b`.
    Add { a: Option<GraphPort>, b: Option<GraphPort> },
    /// `a - b`.
    Subtract { a: Option<GraphPort>, b: Option<GraphPort> },
    /// `a * b`.
    Multiply { a: Option<GraphPort>, b: Option<GraphPort> },
    /// `a / b`.
    Divide { a: Option<GraphPort>, b: Option<GraphPort> },
    /// `sin(a)`.
    Sin { a: Option<GraphPort> },
    /// `cos(a)`.
    Cos { a: Option<GraphPort> },
    /// `abs(a)`.
    Abs { a: Option<GraphPort> },
    /// `sqrt(a)`.
    Sqrt { a: Option<GraphPort> },
    /// `min(a, b)`.
    Min { a: Option<GraphPort>, b: Option<GraphPort> },
    /// `max(a, b)`.
    Max { a: Option<GraphPort>, b: Option<GraphPort> },
    /// Remap `a` from `[from_start, from_end]` to `[to_start, to_end]`.
    Remap {
        a: Option<GraphPort>,
        from_start: f32,
        from_end: f32,
        to_start: f32,
        to_end: f32,
    },
    /// Output sink: writes its single input into the SDF channel of the
    /// destination `VoxelBuffer`. Treated as a leaf in topological order.
    OutputSdf { a: Option<GraphPort> },
}

impl NodeKind {
    /// Returns the input ports this node reads from, in declaration order.
    /// Used by the interpreter to schedule upstream evaluations.
    pub fn inputs(&self) -> Vec<Option<GraphPort>> {
        match self {
            NodeKind::InputX | NodeKind::InputY | NodeKind::InputZ | NodeKind::Constant(_) => {
                Vec::new()
            }
            NodeKind::Add { a, b }
            | NodeKind::Subtract { a, b }
            | NodeKind::Multiply { a, b }
            | NodeKind::Divide { a, b }
            | NodeKind::Min { a, b }
            | NodeKind::Max { a, b } => vec![*a, *b],
            NodeKind::Sin { a }
            | NodeKind::Cos { a }
            | NodeKind::Abs { a }
            | NodeKind::Sqrt { a } => vec![*a],
            NodeKind::Remap { a, .. } => vec![*a],
            NodeKind::OutputSdf { a } => vec![*a],
        }
    }

    /// `true` if this node is an output sink (no downstream consumer in the
    /// graph itself; the runtime materialises its result into a channel).
    pub fn is_output(&self) -> bool {
        matches!(self, NodeKind::OutputSdf { .. })
    }
}

/// A node in the graph.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphNode {
    pub id: GraphNodeId,
    pub kind: NodeKind,
}

impl GraphNode {
    pub fn new(id: GraphNodeId, kind: NodeKind) -> Self {
        Self { id, kind }
    }
}

/// Where the runtime writes the result of an output node. Mirrors the C++
/// `OutputInfo` mapping from a node to a destination channel — the current
/// minimal port supports only the SDF channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphOutput {
    /// Write the output node's result to the SDF channel.
    Sdf,
}

/// A procedural voxel graph. Built incrementally via [`Graph::add_node`].
/// The graph owns no execution state — pass it to [`Graph::generate`] with a
/// per-thread scratch to evaluate.
#[derive(Debug, Clone, Default)]
pub struct Graph {
    nodes: Vec<GraphNode>,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, node: GraphNode) {
        // Defensive: detect duplicate ids so the interpreter's HashMap doesn't
        // silently shadow one. The C++ graph uses a map keyed by id; we keep a
        // Vec but check uniqueness on insert.
        debug_assert!(
            !self.nodes.iter().any(|n| n.id == node.id),
            "duplicate graph node id {:?}",
            node.id
        );
        self.nodes.push(node);
    }

    pub fn nodes(&self) -> &[GraphNode] {
        &self.nodes
    }

    /// Convenience: add a node by kind, picking the next free id.
    pub fn push(&mut self, kind: NodeKind) -> GraphNodeId {
        let id = self.nodes.iter().map(|n| n.id + 1).max().unwrap_or(0);
        self.add_node(GraphNode::new(id, kind));
        id
    }

    /// Returns the ids in topological order (producers before consumers).
    /// The output nodes come last. Returns an error if the graph contains a
    /// cycle or a dangling port reference.
    pub fn topological_order(&self) -> Result<Vec<GraphNodeId>, TopoError> {
        use std::collections::{HashMap, HashSet};

        let by_id: HashMap<GraphNodeId, &GraphNode> =
            self.nodes.iter().map(|n| (n.id, n)).collect();

        let mut visited: HashSet<GraphNodeId> = HashSet::new();
        let mut on_stack: HashSet<GraphNodeId> = HashSet::new();
        let mut order: Vec<GraphNodeId> = Vec::with_capacity(self.nodes.len());

        fn visit(
            id: GraphNodeId,
            by_id: &HashMap<GraphNodeId, &GraphNode>,
            visited: &mut HashSet<GraphNodeId>,
            on_stack: &mut HashSet<GraphNodeId>,
            order: &mut Vec<GraphNodeId>,
        ) -> Result<(), TopoError> {
            if visited.contains(&id) {
                return Ok(());
            }
            if on_stack.contains(&id) {
                return Err(TopoError::Cycle);
            }
            on_stack.insert(id);
            let node = by_id.get(&id).ok_or(TopoError::DanglingPort(id))?;
            for input in node.kind.inputs().into_iter().flatten() {
                visit(input.node, by_id, visited, on_stack, order)?;
            }
            on_stack.remove(&id);
            visited.insert(id);
            order.push(id);
            Ok(())
        }

        // Visit output nodes last so the order ends with them.
        let mut output_ids: Vec<GraphNodeId> = Vec::new();
        for node in &self.nodes {
            if node.kind.is_output() {
                output_ids.push(node.id);
            } else {
                visit(node.id, &by_id, &mut visited, &mut on_stack, &mut order)?;
            }
        }
        for id in output_ids {
            visit(id, &by_id, &mut visited, &mut on_stack, &mut order)?;
        }
        Ok(order)
    }

    /// Evaluate the graph for one Y-slice of voxels. Writes the result of
    /// every output node into the corresponding `outputs` slot; the caller
    /// copies those slices into a `VoxelBuffer` channel.
    ///
    /// `inputs.x`, `inputs.y`, `inputs.z` carry the world-space coordinates
    /// of each voxel in the slice. `slice_size` is `width * depth` (X × Z).
    pub fn generate(
        &self,
        inputs: &GraphInputs<'_>,
        slice_size: usize,
        scratch: &mut GraphScratch,
        outputs: &mut Vec<(GraphOutput, Vec<f32>)>,
    ) -> Result<(), TopoError> {
        let order = self.topological_order()?;
        scratch.clear();
        outputs.clear();

        for id in order {
            let node = self
                .nodes
                .iter()
                .find(|n| n.id == id)
                .expect("topological order contains a node id that is not in the graph");
            match &node.kind {
                NodeKind::InputX => scratch.put(id, inputs.x.to_vec()),
                NodeKind::InputY => scratch.put(id, vec![inputs.y; slice_size]),
                NodeKind::InputZ => scratch.put(id, inputs.z.to_vec()),
                NodeKind::Constant(v) => scratch.put(id, vec![*v; slice_size]),
                NodeKind::Add { a, b } => {
                    let r = binop(scratch, a, b, slice_size, |x, y| x + y);
                    scratch.put(id, r);
                }
                NodeKind::Subtract { a, b } => {
                    let r = binop(scratch, a, b, slice_size, |x, y| x - y);
                    scratch.put(id, r);
                }
                NodeKind::Multiply { a, b } => {
                    let r = binop(scratch, a, b, slice_size, |x, y| x * y);
                    scratch.put(id, r);
                }
                NodeKind::Divide { a, b } => {
                    let r = binop(scratch, a, b, slice_size, |x, y| x / y);
                    scratch.put(id, r);
                }
                NodeKind::Sin { a } => {
                    let r = monop(scratch, a, slice_size, f32::sin);
                    scratch.put(id, r);
                }
                NodeKind::Cos { a } => {
                    let r = monop(scratch, a, slice_size, f32::cos);
                    scratch.put(id, r);
                }
                NodeKind::Abs { a } => {
                    let r = monop(scratch, a, slice_size, f32::abs);
                    scratch.put(id, r);
                }
                NodeKind::Sqrt { a } => {
                    let r = monop(scratch, a, slice_size, f32::sqrt);
                    scratch.put(id, r);
                }
                NodeKind::Min { a, b } => {
                    let r = binop(scratch, a, b, slice_size, f32::min);
                    scratch.put(id, r);
                }
                NodeKind::Max { a, b } => {
                    let r = binop(scratch, a, b, slice_size, f32::max);
                    scratch.put(id, r);
                }
                NodeKind::Remap {
                    a,
                    from_start,
                    from_end,
                    to_start,
                    to_end,
                } => {
                    let from_start = *from_start;
                    let from_end = *from_end;
                    let to_start = *to_start;
                    let to_end = *to_end;
                    let from_span = from_end - from_start;
                    let to_span = to_end - to_start;
                    let r = monop(scratch, a, slice_size, |v| {
                        if from_span.abs() <= f32::EPSILON {
                            to_start
                        } else {
                            let t = (v - from_start) / from_span;
                            to_start + t.clamp(0.0, 1.0) * to_span
                        }
                    });
                    scratch.put(id, r);
                }
                NodeKind::OutputSdf { a } => {
                    let r = monop(scratch, a, slice_size, |v| v);
                    outputs.push((GraphOutput::Sdf, r));
                }
            }
        }

        Ok(())
    }
}

/// Per-thread execution scratch. Stores the f32 slice produced for every
/// node id during a single `generate` call. Reused across calls to avoid
/// reallocation; cleared at the start of each call.
#[derive(Debug, Default)]
pub struct GraphScratch {
    buffers: std::collections::HashMap<GraphNodeId, Vec<f32>>,
}

impl GraphScratch {
    pub fn new() -> Self {
        Self::default()
    }

    fn clear(&mut self) {
        self.buffers.clear();
    }

    fn put(&mut self, id: GraphNodeId, data: Vec<f32>) {
        self.buffers.insert(id, data);
    }

    /// Returns the f32 slice produced by `id`, or `None` if the node hasn't
    /// been evaluated yet. Used internally by the binop/monop helpers.
    fn get(&self, id: GraphNodeId) -> Option<&[f32]> {
        self.buffers.get(&id).map(Vec::as_slice)
    }
}

/// Inputs bound by the caller for one `generate` invocation. `x` and `z`
/// carry per-voxel coordinates for the slice; `y` is the constant slice Y.
#[derive(Debug, Clone, Copy)]
pub struct GraphInputs<'a> {
    pub x: &'a [f32],
    pub y: f32,
    pub z: &'a [f32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopoError {
    Cycle,
    DanglingPort(GraphNodeId),
}

fn binop(
    scratch: &GraphScratch,
    a: &Option<GraphPort>,
    b: &Option<GraphPort>,
    slice_size: usize,
    f: impl Fn(f32, f32) -> f32,
) -> Vec<f32> {
    let a = a
        .as_ref()
        .and_then(|p| scratch.get(p.node))
        .unwrap_or_else(|| panic_zero(slice_size));
    let b = b
        .as_ref()
        .and_then(|p| scratch.get(p.node))
        .unwrap_or_else(|| panic_zero(slice_size));
    debug_assert_eq!(a.len(), slice_size);
    debug_assert_eq!(b.len(), slice_size);
    a.iter().zip(b).map(|(x, y)| f(*x, *y)).collect()
}

fn monop(
    scratch: &GraphScratch,
    a: &Option<GraphPort>,
    slice_size: usize,
    f: impl Fn(f32) -> f32,
) -> Vec<f32> {
    let a = a
        .as_ref()
        .and_then(|p| scratch.get(p.node))
        .unwrap_or_else(|| panic_zero(slice_size));
    debug_assert_eq!(a.len(), slice_size);
    a.iter().map(|v| f(*v)).collect()
}

fn panic_zero(slice_size: usize) -> &'static [f32] {
    static ZERO: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    let buf = ZERO.get_or_init(|| vec![0.0; 4096]);
    if buf.len() >= slice_size {
        &buf[..slice_size]
    } else {
        // Pathological large slice — fall back to a per-call empty slice.
        // The interpreter uses default 0.0 for unconnected inputs; an empty
        // slice is incorrect for slice_size > 0, so we panic to surface the
        // bug rather than silently mis-evaluate.
        panic!(
            "graph input port is unconnected and slice_size {slice_size} exceeds the zero cache"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn x_inputs(slice_size: usize) -> Vec<f32> {
        (0..slice_size).map(|i| i as f32).collect()
    }

    #[test]
    fn topological_order_places_outputs_last() {
        let mut graph = Graph::new();
        let x = graph.push(NodeKind::InputX);
        let c = graph.push(NodeKind::Constant(2.0));
        let mul = graph.push(NodeKind::Multiply {
            a: Some(GraphPort::new(x)),
            b: Some(GraphPort::new(c)),
        });
        let out = graph.push(NodeKind::OutputSdf {
            a: Some(GraphPort::new(mul)),
        });

        let order = graph.topological_order().unwrap();
        assert_eq!(order.last(), Some(&out));
        // The two producers come before the multiply.
        let pos_mul = order.iter().position(|i| *i == mul).unwrap();
        let pos_x = order.iter().position(|i| *i == x).unwrap();
        let pos_c = order.iter().position(|i| *i == c).unwrap();
        assert!(pos_x < pos_mul);
        assert!(pos_c < pos_mul);
    }

    #[test]
    fn multiply_input_x_by_constant_evaluates_correctly() {
        let mut graph = Graph::new();
        let x = graph.push(NodeKind::InputX);
        let c = graph.push(NodeKind::Constant(3.0));
        let mul = graph.push(NodeKind::Multiply {
            a: Some(GraphPort::new(x)),
            b: Some(GraphPort::new(c)),
        });
        graph.push(NodeKind::OutputSdf {
            a: Some(GraphPort::new(mul)),
        });

        let slice = 4;
        let xs = x_inputs(slice);
        let inputs = GraphInputs {
            x: &xs,
            y: 0.0,
            z: &xs,
        };
        let mut scratch = GraphScratch::new();
        let mut outputs = Vec::new();
        graph.generate(&inputs, slice, &mut scratch, &mut outputs).unwrap();

        assert_eq!(outputs.len(), 1);
        let (out_kind, data) = &outputs[0];
        assert_eq!(*out_kind, GraphOutput::Sdf);
        assert_eq!(data, &vec![0.0, 3.0, 6.0, 9.0]);
    }

    #[test]
    fn sin_of_input_x_evaluates_correctly() {
        let mut graph = Graph::new();
        let x = graph.push(NodeKind::InputX);
        let sin = graph.push(NodeKind::Sin {
            a: Some(GraphPort::new(x)),
        });
        graph.push(NodeKind::OutputSdf {
            a: Some(GraphPort::new(sin)),
        });

        let slice = 3;
        let xs = x_inputs(slice);
        let inputs = GraphInputs {
            x: &xs,
            y: 0.0,
            z: &xs,
        };
        let mut scratch = GraphScratch::new();
        let mut outputs = Vec::new();
        graph.generate(&inputs, slice, &mut scratch, &mut outputs).unwrap();

        let (_, data) = &outputs[0];
        for (i, v) in data.iter().enumerate() {
            assert!((v - (i as f32).sin()).abs() < 1e-5, "sin mismatch at {i}");
        }
    }

    #[test]
    fn remap_clamps_outside_the_input_range() {
        let mut graph = Graph::new();
        let x = graph.push(NodeKind::InputX);
        let remap = graph.push(NodeKind::Remap {
            a: Some(GraphPort::new(x)),
            from_start: 0.0,
            from_end: 2.0,
            to_start: 10.0,
            to_end: 20.0,
        });
        graph.push(NodeKind::OutputSdf {
            a: Some(GraphPort::new(remap)),
        });

        // 0 -> 10, 1 -> 15, 2 -> 20, 5 -> clamps to 20.
        let xs = vec![0.0, 1.0, 2.0, 5.0];
        let inputs = GraphInputs {
            x: &xs,
            y: 0.0,
            z: &xs,
        };
        let mut scratch = GraphScratch::new();
        let mut outputs = Vec::new();
        graph.generate(&inputs, 4, &mut scratch, &mut outputs).unwrap();

        let (_, data) = &outputs[0];
        assert!((data[0] - 10.0).abs() < 1e-5);
        assert!((data[1] - 15.0).abs() < 1e-5);
        assert!((data[2] - 20.0).abs() < 1e-5);
        assert!((data[3] - 20.0).abs() < 1e-5);
    }

    #[test]
    fn cycle_in_the_graph_returns_an_error() {
        let mut graph = Graph::new();
        // Two nodes feeding each other. The GraphPort indirection doesn't
        // require the producer to exist, so we can construct a pure cycle
        // directly via NodeKind::Add references.
        let a = graph.push(NodeKind::Add {
            a: Some(GraphPort::new(1)),
            b: None,
        });
        let _ = a;
        // Build an actual self-cycle: a -> b -> a.
        let mut cycle_graph = Graph::new();
        cycle_graph.add_node(GraphNode::new(
            1,
            NodeKind::Add {
                a: Some(GraphPort::new(2)),
                b: None,
            },
        ));
        cycle_graph.add_node(GraphNode::new(
            2,
            NodeKind::Add {
                a: Some(GraphPort::new(1)),
                b: None,
            },
        ));
        let result = cycle_graph.topological_order();
        assert_eq!(result.unwrap_err(), TopoError::Cycle);
    }

    #[test]
    fn unconnected_input_defaults_to_zero() {
        let mut graph = Graph::new();
        // Add with both inputs unconnected — equivalent to 0 + 0.
        let add = graph.push(NodeKind::Add {
            a: None,
            b: None,
        });
        graph.push(NodeKind::OutputSdf {
            a: Some(GraphPort::new(add)),
        });

        let xs = vec![0.0; 2];
        let inputs = GraphInputs {
            x: &xs,
            y: 0.0,
            z: &xs,
        };
        let mut scratch = GraphScratch::new();
        let mut outputs = Vec::new();
        graph.generate(&inputs, 2, &mut scratch, &mut outputs).unwrap();
        let (_, data) = &outputs[0];
        assert_eq!(data, &vec![0.0, 0.0]);
    }

    #[test]
    fn node_kind_inputs_lists_consumer_ports_in_order() {
        let kind = NodeKind::Add {
            a: Some(GraphPort::new(1)),
            b: Some(GraphPort::new(2)),
        };
        let inputs = kind.inputs();
        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs[0].unwrap().node, 1);
        assert_eq!(inputs[1].unwrap().node, 2);
    }

    #[test]
    fn graph_node_id_can_be_user_supplied() {
        let mut graph = Graph::new();
        graph.add_node(GraphNode::new(100, NodeKind::Constant(1.0)));
        graph.add_node(GraphNode::new(200, NodeKind::Constant(2.0)));
        assert_eq!(graph.nodes().len(), 2);
        let order = graph.topological_order().unwrap();
        assert!(order.contains(&100));
        assert!(order.contains(&200));
    }

    #[test]
    fn slice_z_coordinates_round_trip_through_input_z() {
        let mut graph = Graph::new();
        let z = graph.push(NodeKind::InputZ);
        graph.push(NodeKind::OutputSdf {
            a: Some(GraphPort::new(z)),
        });

        let zs = vec![10.0, 20.0, 30.0];
        let inputs = GraphInputs {
            x: &zs,
            y: 0.0,
            z: &zs,
        };
        let mut scratch = GraphScratch::new();
        let mut outputs = Vec::new();
        graph.generate(&inputs, 3, &mut scratch, &mut outputs).unwrap();
        let (_, data) = &outputs[0];
        assert_eq!(data, &zs);
    }
}
