//! Bounded, typed expressions and exact cell tracks retained by scene properties.
//! Playback evaluates this DAG, never source-language syntax or user functions.
use crate::{Easing, Fraction, Number, Property, Track, Type, Unit, Value, math};
use imbl::{OrdMap, Vector};
use serde::{Deserialize, Serialize};

pub type CellId = u64;
pub type NodeId = usize;
pub const MAX_REACTIVE_NODES: usize = 65_536;
pub const MAX_REACTIVE_DEPTH: usize = 128;
pub const MAX_REACTIVE_TRACKS: usize = 65_536;
pub const MAX_REACTIVE_BINDINGS: usize = 65_536;
pub const MAX_REACTIVE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Constant(Value),
    Cell(CellId),
    Unary {
        op: String,
        value: NodeId,
    },
    Binary {
        op: String,
        left: NodeId,
        right: NodeId,
    },
    Math {
        name: String,
        args: Vec<NodeId>,
    },
    Vector {
        values: Vec<NodeId>,
    },
    Component {
        vector: NodeId,
        index: usize,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Expression {
    operation: Node,
    ty: Type,
    depth: usize,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellChannel {
    pub initial: Value,
    pub tracks: Vector<Track>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "GraphData")]
pub struct ReactiveGraph {
    nodes: Vector<Expression>,
    pub(crate) cells: OrdMap<CellId, CellChannel>,
    pub(crate) bindings: OrdMap<String, OrdMap<Property, NodeId>>,
    #[serde(skip)]
    track_count: usize,
    #[serde(skip)]
    binding_count: usize,
    #[serde(skip)]
    metadata_bytes: usize,
}
#[derive(Deserialize)]
struct GraphData {
    nodes: Vector<Expression>,
    cells: OrdMap<CellId, CellChannel>,
    bindings: OrdMap<String, OrdMap<Property, NodeId>>,
}
impl TryFrom<GraphData> for ReactiveGraph {
    type Error = String;
    fn try_from(data: GraphData) -> Result<Self, String> {
        let mut graph = Self {
            nodes: data.nodes,
            cells: data.cells,
            bindings: data.bindings,
            track_count: 0,
            binding_count: 0,
            metadata_bytes: 0,
        };
        (graph.track_count, graph.binding_count, graph.metadata_bytes) =
            graph.validated_counts()?;
        Ok(graph)
    }
}
impl ReactiveGraph {
    pub fn has_cell(&self, id: CellId) -> bool {
        self.cells.contains_key(&id)
    }
    pub fn has_bindings(&self) -> bool {
        !self.bindings.is_empty()
    }
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
    pub fn node_type(&self, id: NodeId) -> Option<&Type> {
        self.nodes.get(id).map(|node| &node.ty)
    }
    /// Conservative leaf-cell dependencies for preparation-time property getters.
    /// A bounded walk avoids duplicating dependency sets on every retained node.
    pub fn cell_dependencies(&self, root: NodeId) -> Result<imbl::OrdSet<CellId>, String> {
        self.dependency_cells(vec![root])
    }
    pub(crate) fn bound_cells(
        &self,
        mut visible: impl FnMut(&str) -> bool,
    ) -> imbl::OrdSet<CellId> {
        let roots = self
            .bindings
            .iter()
            .filter(|(id, _)| visible(id))
            .flat_map(|(_, properties)| properties.values().copied())
            .collect();
        self.dependency_cells(roots)
            .expect("retained bindings reference a validated graph")
    }
    fn dependency_cells(&self, mut pending: Vec<NodeId>) -> Result<imbl::OrdSet<CellId>, String> {
        let mut visited = std::collections::BTreeSet::new();
        let mut cells = imbl::OrdSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            let expression = self
                .nodes
                .get(id)
                .ok_or("retained dependency lookup references a missing node")?;
            match &expression.operation {
                Node::Cell(id) => {
                    cells.insert(*id);
                }
                Node::Constant(_) => {}
                Node::Unary { value, .. } => pending.push(*value),
                Node::Binary { left, right, .. } => {
                    pending.push(*left);
                    pending.push(*right);
                }
                Node::Math { args, .. } => pending.extend(args),
                Node::Vector { values } => pending.extend(values),
                Node::Component { vector, .. } => pending.push(*vector),
            }
        }
        Ok(cells)
    }
    pub fn metadata_bytes(&self) -> usize {
        self.metadata_bytes
    }
    fn reserve(&self, additional: usize) -> Result<usize, String> {
        let bytes = self
            .metadata_bytes
            .checked_add(additional)
            .ok_or("retained metadata size overflow")?;
        if bytes > MAX_REACTIVE_BYTES {
            Err("retained expression/cell/property metadata exceeds its byte budget".into())
        } else {
            Ok(bytes)
        }
    }
    pub fn binding(&self, element: &str, property: Property) -> Option<NodeId> {
        self.bindings.get(element)?.get(&property).copied()
    }
    pub fn properties(&self, element: &str) -> Option<&OrdMap<Property, NodeId>> {
        self.bindings.get(element)
    }
    pub fn element_bindings(&self) -> &OrdMap<String, OrdMap<Property, NodeId>> {
        &self.bindings
    }
    /// Register the initial value once. Later program writes are exact timed events.
    pub fn set_cell(&mut self, id: CellId, value: Value) -> Result<(), String> {
        leaf_type(&value)?;
        if self.cells.contains_key(&id) {
            return Err(format!("reactive cell {id} is already registered"));
        }
        if self.cells.len() >= MAX_REACTIVE_NODES {
            return Err("retained reactive cell limit exceeded".into());
        }
        let bytes =
            self.reserve(std::mem::size_of::<CellChannel>() + std::mem::size_of::<CellId>())?;
        self.cells.insert(
            id,
            CellChannel {
                initial: value,
                tracks: Vector::new(),
            },
        );
        self.metadata_bytes = bytes;
        Ok(())
    }
    pub fn sample_cell(&self, id: CellId, time: Fraction) -> Result<Value, String> {
        check_time(time)?;
        let cell = self
            .cells
            .get(&id)
            .ok_or_else(|| format!("unknown reactive cell {id}"))?;
        Track::at(&cell.tracks, time)
            .map_or_else(|| Ok(cell.initial.clone()), |track| track.sample(time))
    }
    pub fn write_cell(&mut self, id: CellId, time: Fraction, value: Value) -> Result<(), String> {
        self.insert_cell_track(id, time, time, value, Easing::Linear)
    }
    pub fn animate_cell(
        &mut self,
        id: CellId,
        start: Fraction,
        end: Fraction,
        to: Value,
        easing: Easing,
    ) -> Result<(), String> {
        if end <= start {
            return Err("cell animation requires a positive duration".into());
        }
        self.insert_cell_track(id, start, end, to, easing)
    }
    fn insert_cell_track(
        &mut self,
        id: CellId,
        start: Fraction,
        end: Fraction,
        to: Value,
        easing: Easing,
    ) -> Result<(), String> {
        check_time(start)?;
        check_time(end)?;
        math::timeline_end(start, math::arithmetic("-", end, start)?)?;
        let cell = self
            .cells
            .get(&id)
            .ok_or_else(|| format!("unknown reactive cell {id}"))?;
        if leaf_type(&cell.initial)? != leaf_type(&to)? {
            return Err(
                "cell assignment/animation must preserve its exact value type and units".into(),
            );
        }
        // Insert after equal-time events: the last source write wins at an instant.
        let index = cell
            .tracks
            .binary_search_by(|track| {
                if track.start <= start {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            })
            .unwrap_err();
        let previous = index
            .checked_sub(1)
            .and_then(|index| cell.tracks.get(index));
        let next = cell.tracks.get(index);
        if previous.is_some_and(|track| track.end > start)
            || next.is_some_and(|track| track.start < end)
        {
            return Err(format!(
                "overlapping writes for reactive cell {id}; schedule each writer outside the other animation's interval"
            ));
        }
        if self.track_count >= MAX_REACTIVE_TRACKS {
            return Err("retained cell-track limit exceeded".into());
        }
        let from = self.sample_cell(id, start)?;
        let track = Track {
            start,
            end,
            from,
            to,
            easing,
            reveal: None,
            horizontal: None,
        };
        // An animation's implicit starting value comes from its chronological
        // predecessor, even when that earlier write was prepared later in source order.
        let successor = next.map(|next| {
            let mut next = next.clone();
            next.from = track.to.clone();
            next
        });
        for candidate in std::iter::once(&track).chain(successor.as_ref()) {
            if candidate.end > candidate.start {
                candidate.sample(math::lerp(
                    candidate.start,
                    candidate.end,
                    Fraction::new(1u64, 2u64),
                )?)?;
            }
        }
        let bytes = self.reserve(2 * std::mem::size_of::<Track>())?;
        let mut tracks = cell.tracks.clone();
        tracks.insert(index, track);
        if let Some(successor) = successor {
            tracks[index + 1] = successor;
        }
        self.cells
            .get_mut(&id)
            .expect("validated reactive cell")
            .tracks = tracks;
        self.track_count += 1;
        self.metadata_bytes = bytes;
        Ok(())
    }
    pub fn insert_node(&mut self, operation: Node, limit: usize) -> Result<NodeId, String> {
        if self.nodes.len() >= limit.min(MAX_REACTIVE_NODES) {
            return Err("retained expression node limit exceeded; simplify the signal or increase the host's node budget".into());
        }
        let id = self.nodes.len();
        let (ty, depth) = self.describe(&operation, id)?;
        if depth > MAX_REACTIVE_DEPTH {
            return Err("retained expression dependency depth exceeded".into());
        }
        let bytes = self.reserve(node_bytes(&operation))?;
        self.nodes.push_back(Expression {
            operation,
            ty,
            depth,
        });
        self.metadata_bytes = bytes;
        Ok(id)
    }
    pub fn bind(
        &mut self,
        element: String,
        property: Property,
        node: NodeId,
    ) -> Result<(), String> {
        if element.is_empty() {
            return Err("retained properties require a nonempty element identity".into());
        }
        let expression = self
            .nodes
            .get(node)
            .ok_or("retained property references a missing expression")?;
        property_type(property, &expression.ty)?;
        if let Some(previous) = self.binding(&element, property) {
            return if previous == node {
                Ok(())
            } else {
                Err(format!(
                    "multiple retained writers for {element}.{property:?}"
                ))
            };
        }
        if self.binding_count >= MAX_REACTIVE_BINDINGS {
            return Err("retained property binding limit exceeded".into());
        }
        let additional = if self.bindings.contains_key(&element) {
            0
        } else {
            element.len() + std::mem::size_of::<String>()
        } + 2 * (std::mem::size_of::<Property>() + std::mem::size_of::<NodeId>());
        let bytes = self.reserve(additional)?;
        self.bindings
            .entry(element)
            .or_default()
            .insert(property, node);
        self.binding_count += 1;
        self.metadata_bytes = bytes;
        Ok(())
    }
    pub fn sampler(&self, time: Fraction) -> Result<Sampler<'_>, String> {
        check_time(time)?;
        Ok(Sampler {
            graph: self,
            time,
            values: vec![None; self.nodes.len()],
            evaluated: 0,
        })
    }
    fn describe(&self, operation: &Node, before: usize) -> Result<(Type, usize), String> {
        let child = |id: NodeId| -> Result<&Expression, String> {
            if id >= before {
                return Err("retained expression must reference earlier nodes; forward references and cycles are forbidden".into());
            }
            self.nodes
                .get(id)
                .ok_or_else(|| "retained expression references a missing node".into())
        };
        match operation {
            Node::Constant(value) => Ok((leaf_type(value)?, 1)),
            Node::Cell(id) => Ok((
                leaf_type(
                    &self
                        .cells
                        .get(id)
                        .ok_or("retained expression references a missing cell")?
                        .initial,
                )?,
                1,
            )),
            Node::Unary { op, value } => {
                let value = child(*value)?;
                Ok((
                    math::operations::unary_type(op, &value.ty)?,
                    value.depth + 1,
                ))
            }
            Node::Binary { op, left, right } => {
                let left = child(*left)?;
                let right = child(*right)?;
                Ok((
                    math::operations::binary_type(op, &left.ty, &right.ty)?,
                    left.depth.max(right.depth) + 1,
                ))
            }
            Node::Vector { values } => {
                if !(2..=4).contains(&values.len()) {
                    return Err(
                        "retained vectors require two, three or four numeric components".into(),
                    );
                }
                let mut unit = None;
                let mut depth = 0;
                for &id in values {
                    let value = child(id)?;
                    let component = math::operations::scalar_unit(&value.ty)
                        .ok_or("retained vector components must be numbers")?;
                    if unit.is_some_and(|unit| unit != component) {
                        return Err("retained vector components must use matching units".into());
                    }
                    unit = Some(component);
                    depth = depth.max(value.depth);
                }
                Ok((Type::Vector(values.len(), unit), depth + 1))
            }
            Node::Component { vector, index } => {
                let vector = child(*vector)?;
                let Type::Vector(dimension, Some(unit)) = &vector.ty else {
                    return Err(
                        "retained component access requires a vector with known units".into(),
                    );
                };
                if *index >= *dimension {
                    return Err("retained vector component index is out of range".into());
                }
                Ok((Type::Numeric(*unit), vector.depth + 1))
            }
            Node::Math { name, args } => {
                let function = math::functions::function(name)
                    .ok_or_else(|| format!("'{name}' is not a retained math function"))?;
                let required = function
                    .parameters
                    .iter()
                    .filter(|(_, required)| *required)
                    .count();
                if args.len() < required || args.len() > function.parameters.len() {
                    return Err(format!(
                        "retained {name} expects {required}..={} arguments",
                        function.parameters.len()
                    ));
                }
                let mut depth = 0;
                let mut first = None;
                for &id in args {
                    let node = child(id)?;
                    depth = depth.max(node.depth);
                    let unit = math::operations::scalar_unit(&node.ty)
                        .ok_or("retained math requires numeric arguments")?;
                    if let Some(first) = first {
                        if function.domain == math::functions::Domain::Numeric && first != unit {
                            return Err(
                                "retained numeric function arguments require matching units".into(),
                            );
                        }
                    } else {
                        first = Some(unit);
                    }
                    match function.domain {
                        math::functions::Domain::Scalar if unit != Unit::Number => {
                            return Err(
                                "retained scalar math requires dimensionless arguments".into()
                            );
                        }
                        math::functions::Domain::Trig
                            if !matches!(unit, Unit::Number | Unit::Angle) =>
                        {
                            return Err("retained trigonometry requires Number or Angle".into());
                        }
                        _ => {}
                    }
                }
                let unit = if function.domain == math::functions::Domain::Numeric && name != "sign"
                {
                    first.expect("required math argument")
                } else {
                    Unit::Number
                };
                Ok((Type::Numeric(unit), depth + 1))
            }
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let counts = self.validated_counts()?;
        if counts != (self.track_count, self.binding_count, self.metadata_bytes) {
            return Err("invalid retained metadata accounting".into());
        }
        Ok(())
    }
    fn validated_counts(&self) -> Result<(usize, usize, usize), String> {
        if self.nodes.len() > MAX_REACTIVE_NODES || self.cells.len() > MAX_REACTIVE_NODES {
            return Err("retained reactive node/cell limit exceeded".into());
        }
        let mut tracks = 0usize;
        for cell in self.cells.values() {
            let ty = leaf_type(&cell.initial)?;
            let mut previous: Option<Fraction> = None;
            let mut previous_value = &cell.initial;
            for track in &cell.tracks {
                check_time(track.start)?;
                check_time(track.end)?;
                math::timeline_end(track.start, math::arithmetic("-", track.end, track.start)?)?;
                if previous.is_some_and(|end| end > track.start)
                    || track.reveal.is_some()
                    || track.horizontal.is_some()
                {
                    return Err("invalid or overlapping retained cell tracks".into());
                }
                if leaf_type(&track.from)? != ty || leaf_type(&track.to)? != ty {
                    return Err("retained track endpoint types differ from its cell".into());
                }
                if &track.from != previous_value {
                    return Err(
                        "retained track starting value differs from its chronological predecessor"
                            .into(),
                    );
                }
                if track.end > track.start {
                    track.sample(math::lerp(
                        track.start,
                        track.end,
                        Fraction::new(1u64, 2u64),
                    )?)?;
                }
                previous = Some(track.end);
                previous_value = &track.to;
                tracks = tracks
                    .checked_add(1)
                    .ok_or("retained track count overflow")?;
                if tracks > MAX_REACTIVE_TRACKS {
                    return Err("retained cell-track limit exceeded".into());
                }
            }
        }
        for (index, node) in self.nodes.iter().enumerate() {
            let (ty, depth) = self.describe(&node.operation, index)?;
            if ty != node.ty || depth != node.depth || depth > MAX_REACTIVE_DEPTH {
                return Err("invalid retained expression type/depth metadata".into());
            }
        }
        let mut bindings = 0usize;
        for (id, properties) in &self.bindings {
            if id.is_empty() {
                return Err("retained element identity is empty".into());
            }
            for (property, node) in properties {
                property_type(
                    *property,
                    &self
                        .nodes
                        .get(*node)
                        .ok_or("retained property references a missing node")?
                        .ty,
                )?;
                bindings += 1;
                if bindings > MAX_REACTIVE_BINDINGS {
                    return Err("retained property binding limit exceeded".into());
                }
            }
        }
        let bytes = self
            .nodes
            .iter()
            .map(|node| node_bytes(&node.operation))
            .sum::<usize>()
            + self.cells.len()
                * (std::mem::size_of::<CellChannel>() + std::mem::size_of::<CellId>())
            + tracks * 2 * std::mem::size_of::<Track>()
            + self
                .bindings
                .keys()
                .map(|id| id.len() + std::mem::size_of::<String>())
                .sum::<usize>()
            + bindings * 2 * (std::mem::size_of::<Property>() + std::mem::size_of::<NodeId>());
        if bytes > MAX_REACTIVE_BYTES {
            return Err("invalid or oversized retained metadata accounting".into());
        }
        Ok((tracks, bindings, bytes))
    }
    pub(crate) fn changed_from(&self, next: &Self) -> Option<Fraction> {
        if self.bindings.is_empty() && next.bindings.is_empty() {
            return None;
        }
        if self.nodes != next.nodes || self.bindings != next.bindings {
            return Some(Fraction::from(0));
        }
        let mut earliest: Option<Fraction> = None;
        for id in self.cells.keys().chain(next.cells.keys()) {
            let (Some(a), Some(b)) = (self.cells.get(id), next.cells.get(id)) else {
                return Some(Fraction::from(0));
            };
            if a.initial != b.initial {
                return Some(Fraction::from(0));
            }
            if a.tracks == b.tracks {
                continue;
            }
            let common = a
                .tracks
                .iter()
                .zip(&b.tracks)
                .take_while(|(a, b)| a == b)
                .count();
            for track in a.tracks.get(common).into_iter().chain(b.tracks.get(common)) {
                earliest = Some(earliest.map_or(track.start, |time| time.min(track.start)));
            }
        }
        earliest
    }
}
fn node_bytes(node: &Node) -> usize {
    std::mem::size_of::<Expression>()
        + match node {
            Node::Unary { op, .. } | Node::Binary { op, .. } => op.len(),
            Node::Math { name, args } => name.len() + args.len() * std::mem::size_of::<NodeId>(),
            Node::Vector { values } => values.len() * std::mem::size_of::<NodeId>(),
            _ => 0,
        }
}

/// A frame-local memo shared by every retained property in a scene sample.
pub struct Sampler<'a> {
    graph: &'a ReactiveGraph,
    time: Fraction,
    values: Vec<Option<Value>>,
    evaluated: usize,
}
impl Sampler<'_> {
    pub fn evaluated_nodes(&self) -> usize {
        self.evaluated
    }
    pub fn value(&mut self, id: NodeId) -> Result<Value, String> {
        if let Some(value) = self.values.get(id).and_then(Option::as_ref) {
            return Ok(value.clone());
        }
        let graph = self.graph;
        let node = &graph
            .nodes
            .get(id)
            .ok_or("retained sampler references a missing node")?
            .operation;
        let value = match node {
            Node::Constant(value) => value.clone(),
            Node::Cell(cell) => self.graph.sample_cell(*cell, self.time)?,
            Node::Unary { op, value } => math::operations::unary(op, self.value(*value)?)?,
            Node::Binary { op, left, right } => {
                let left = self.value(*left)?;
                if matches!(op.as_str(), "and" | "or")
                    && (op == "and" && !left.boolean()? || op == "or" && left.boolean()?)
                {
                    left
                } else {
                    math::operations::binary(op, left, self.value(*right)?)?
                }
            }
            Node::Math { name, args } => {
                let numbers = args
                    .iter()
                    .map(|id| self.value(*id)?.number())
                    .collect::<Result<Vec<Number>, String>>()?;
                Value::Number(math::functions::evaluate(name, &numbers)?)
            }
            Node::Vector { values } => {
                let numbers = values
                    .iter()
                    .map(|id| self.value(*id)?.number())
                    .collect::<Result<Vec<_>, String>>()?;
                let unit = numbers[0].unit;
                let components = numbers
                    .iter()
                    .map(|value| math::to_f64(value.value))
                    .collect::<Result<Vec<_>, _>>()?;
                Value::from_vector(components, unit)?
            }
            Node::Component { vector, index } => {
                let vector = self.value(*vector)?;
                let (values, unit) = vector.vector()?;
                Value::Number(Number {
                    value: math::from_f64(values[*index])?,
                    unit,
                })
            }
        };
        leaf_type(&value)?;
        self.values[id] = Some(value.clone());
        self.evaluated += 1;
        Ok(value)
    }
}
fn check_time(time: Fraction) -> Result<(), String> {
    if !math::finite(&time) || time < Fraction::from(0) {
        Err("retained sample time must be finite and nonnegative".into())
    } else {
        Ok(())
    }
}
fn leaf_type(value: &Value) -> Result<Type, String> {
    match value {
        Value::Number(number) if math::finite(&number.value) => {},
        Value::Fraction(value) if math::finite(value) => {},
        Value::Bool(_) => {},
        Value::Color(color) if color.0.iter().all(|v| math::finite(v) && *v >= Fraction::from(0) && *v <= Fraction::from(1)) => {},
        Value::Vec2(_) | Value::Vec3(_) | Value::Vec4(_) if value.vector()?.0.iter().all(|v| v.is_finite()) => {},
        _ => return Err("retained expressions require finite scalar, boolean, color or vector values; materialize other data with let first".into()),
    }
    Ok(value.ty())
}
fn property_type(property: Property, ty: &Type) -> Result<(), String> {
    let valid = match property {
        Property::Position => matches!(ty, Type::Vector(2, Some(Unit::Length | Unit::Percent))),
        Property::Rotation => math::operations::scalar_unit(ty) == Some(Unit::Angle),
        Property::Scale | Property::StretchX | Property::Opacity | Property::Reveal => {
            math::operations::scalar_unit(ty) == Some(Unit::Number)
        }
        Property::Radius | Property::Width | Property::Height => {
            math::operations::scalar_unit(ty) == Some(Unit::Length)
        }
        Property::Fill => *ty == Type::Color,
    };
    if valid {
        Ok(())
    } else {
        Err(format!("retained {property:?} has incompatible type {ty}"))
    }
}
