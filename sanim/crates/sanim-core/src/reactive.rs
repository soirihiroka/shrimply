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
    Template {
        parts: std::sync::Arc<[crate::TemplatePart<NodeId>]>,
    },
    Vector {
        values: Vec<NodeId>,
    },
    Component {
        vector: NodeId,
        index: usize,
    },
    List {
        values: Vec<NodeId>,
    },
    Record {
        fields: std::collections::BTreeMap<String, NodeId>,
    },
    NativeConfig {
        operation: crate::code::values::Native,
        arguments: std::collections::BTreeMap<String, NodeId>,
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
    pub ty: std::sync::Arc<Type>,
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
                Node::Template { parts } => {
                    for part in parts.iter() {
                        if let crate::TemplatePart::Hole { value, format, .. } = part {
                            pending.push(*value);
                            pending.extend(format);
                        }
                    }
                }
                Node::Vector { values } => pending.extend(values),
                Node::List { values } => pending.extend(values),
                Node::Record { fields } => pending.extend(fields.values()),
                Node::NativeConfig { arguments, .. } => pending.extend(arguments.values()),
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
        self.set_typed_cell(id, value.ty(), value)
    }
    /// Retain the declared schema, including union alternatives and empty Option/List element types.
    pub fn set_typed_cell(&mut self, id: CellId, ty: Type, value: Value) -> Result<(), String> {
        let schema_bytes = state_schema_bytes(&ty)?;
        let value_bytes = state_value_bytes(&ty, &value)?;
        if self.cells.contains_key(&id) {
            return Err(format!("reactive cell {id} is already registered"));
        }
        if self.cells.len() >= MAX_REACTIVE_NODES {
            return Err("retained reactive cell limit exceeded".into());
        }
        let bytes = self.reserve(
            std::mem::size_of::<CellChannel>()
                + std::mem::size_of::<CellId>()
                + schema_bytes
                + value_bytes,
        )?;
        self.cells.insert(
            id,
            CellChannel {
                ty: std::sync::Arc::new(ty),
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
        Track::at(&cell.tracks, time).map_or_else(
            || Ok(cell.initial.clone()),
            |track| {
                if interpolated_type(&cell.ty) {
                    track.sample(time)
                } else if time >= track.end {
                    Ok(track.to.clone())
                } else {
                    Ok(track.from.clone())
                }
            },
        )
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
        let value_bytes = state_value_bytes(&cell.ty, &to).map_err(|e| {
            format!(
                "cell assignment/animation must preserve its declared type {}: {e}",
                cell.ty
            )
        })?;
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
            bounded: false,
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
            if interpolated_type(&cell.ty) && candidate.end > candidate.start {
                candidate.sample(math::lerp(
                    candidate.start,
                    candidate.end,
                    Fraction::new(1u64, 2u64),
                )?)?;
            }
        }
        let bytes = self.reserve(2 * std::mem::size_of::<Track>() + value_bytes)?;
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
        let bytes = self.reserve(node_bytes(&operation)?)?;
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
            Node::NativeConfig {
                operation,
                arguments,
            } => {
                let mut depth = 0;
                let actuals = arguments
                    .iter()
                    .map(|(name, id)| {
                        let value = child(*id)?;
                        depth = depth.max(value.depth);
                        Ok((name.clone(), value.ty.clone()))
                    })
                    .collect::<Result<std::collections::BTreeMap<_, _>, String>>()?;
                Ok((operation.check(&actuals)?, depth + 1))
            }
            Node::Constant(value) => Ok((leaf_type(value)?, 1)),
            Node::List { values } => {
                let mut depth = 0;
                let mut types = Vec::with_capacity(values.len());
                for id in values {
                    let child = child(*id)?;
                    depth = depth.max(child.depth);
                    types.push(child.ty.clone());
                }
                let element = if types.is_empty() {
                    Type::Unknown
                } else {
                    Type::union(types)?
                };
                Ok((
                    Type::List(Box::new(element), crate::ListLength::Exact(values.len())),
                    depth + 1,
                ))
            }
            Node::Record { fields } => {
                let mut depth = 0;
                let mut types = std::collections::BTreeMap::new();
                for (name, id) in fields {
                    let child = child(*id)?;
                    depth = depth.max(child.depth);
                    types.insert(name.clone(), child.ty.clone());
                }
                Ok((Type::Record(types), depth + 1))
            }
            Node::Cell(id) => {
                let cell = self
                    .cells
                    .get(id)
                    .ok_or("retained expression references a missing cell")?;
                if !matches!(
                    cell.ty.as_ref(),
                    Type::Numeric(_)
                        | Type::RefinedNumeric(_)
                        | Type::Fraction
                        | Type::Bool
                        | Type::String
                        | Type::StringLiteral(_)
                        | Type::Union(_)
                        | Type::Color
                        | Type::Font
                        | Type::Code(_)
                        | Type::List(_, _)
                        | Type::Record(_)
                        | Type::Option(_)
                        | Type::Enum { .. }
                        | Type::Vector(2..=4, Some(_))
                ) {
                    return Err(format!(
                        "state of type {} cannot be used by a native animated argument; use scalar, font, code, enum or structured configuration values",
                        cell.ty
                    ));
                }
                Ok((cell.ty.as_ref().clone(), 1))
            }
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
            Node::Template { parts } => {
                let mut depth = 0;
                for part in parts.iter() {
                    if let crate::TemplatePart::Hole {
                        value,
                        format,
                        conversion,
                    } = part
                    {
                        depth = depth.max(child(*value)?.depth);
                        if let Some(format) = format {
                            let format = child(*format)?;
                            if !Type::String.accepts(&format.ty) {
                                return Err("retained template format requires a String".into());
                            }
                            depth = depth.max(format.depth);
                        }
                        if conversion.is_some_and(|value| !matches!(value, 's' | 'r' | 'a')) {
                            return Err("retained template conversion requires s, r or a".into());
                        }
                    }
                }
                Ok((Type::String, depth + 1))
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
        let mut state_bytes = 0usize;
        for cell in self.cells.values() {
            let schema = state_schema_bytes(&cell.ty)?;
            let initial = state_value_bytes(&cell.ty, &cell.initial)?;
            state_bytes = state_bytes
                .checked_add(schema)
                .and_then(|bytes| bytes.checked_add(initial))
                .ok_or("retained state size overflow")?;
            if state_bytes > MAX_REACTIVE_BYTES {
                return Err("retained state exceeds its byte budget".into());
            }
            let mut previous: Option<Fraction> = None;
            let mut previous_value = &cell.initial;
            for track in &cell.tracks {
                check_time(track.start)?;
                check_time(track.end)?;
                math::timeline_end(track.start, math::arithmetic("-", track.end, track.start)?)?;
                if previous.is_some_and(|end| end > track.start)
                    || track.reveal.is_some()
                    || track.horizontal.is_some()
                    || track.bounded
                {
                    return Err("invalid or overlapping retained cell tracks".into());
                }
                crate::validate_value(&cell.ty, &track.from)?;
                let target = state_value_bytes(&cell.ty, &track.to)?;
                state_bytes = state_bytes
                    .checked_add(target)
                    .ok_or("retained state size overflow")?;
                if state_bytes > MAX_REACTIVE_BYTES {
                    return Err("retained state exceeds its byte budget".into());
                }
                if &track.from != previous_value {
                    return Err(
                        "retained track starting value differs from its chronological predecessor"
                            .into(),
                    );
                }
                if interpolated_type(&cell.ty) && track.end > track.start {
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
        let bytes = self.nodes.iter().try_fold(0usize, |bytes, node| {
            bytes
                .checked_add(node_bytes(&node.operation)?)
                .ok_or_else(|| "retained metadata accounting overflow".to_owned())
        })? + self.cells.len()
            * (std::mem::size_of::<CellChannel>() + std::mem::size_of::<CellId>())
            + tracks * 2 * std::mem::size_of::<Track>()
            + state_bytes
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
    /// Visit changed property bindings reachable from the supplied scene roots.
    /// Shared expression pairs are compared once, with bounded diff metadata.
    /// Times are clamped to the element's birth, including future live-list items.
    pub fn visit_changed_bindings(
        &self,
        next: &Self,
        mut birth: impl FnMut(&str) -> Option<Fraction>,
        mut visit: impl FnMut(&str, Property, Fraction),
    ) {
        let mut memo = std::collections::HashMap::new();
        let mut compare = |id: &str,
                           a: Option<&OrdMap<Property, NodeId>>,
                           b: Option<&OrdMap<Property, NodeId>>| {
            let Some(birth) = birth(id) else { return };
            for property in a
                .into_iter()
                .flat_map(|properties| properties.keys())
                .chain(
                    b.into_iter()
                        .flat_map(|properties| properties.keys())
                        .filter(|property| {
                            a.is_none_or(|properties| !properties.contains_key(*property))
                        }),
                )
            {
                let changed = match (
                    a.and_then(|properties| properties.get(property)),
                    b.and_then(|properties| properties.get(property)),
                ) {
                    (Some(&a), Some(&b)) => self.node_changed_from(next, a, b, &mut memo),
                    _ => Some(Fraction::from(0)),
                };
                if let Some(time) = changed {
                    visit(id, *property, time.max(birth));
                }
            }
        };
        // Both maps are ordered: merge once rather than looking up every
        // binding in the opposite tree (and visiting shared IDs twice).
        let mut old = self.bindings.iter().peekable();
        let mut new = next.bindings.iter().peekable();
        loop {
            match (old.peek(), new.peek()) {
                (Some((a, _)), Some((b, _))) if a == b => {
                    let (id, a) = old.next().expect("peeked binding");
                    let (_, b) = new.next().expect("peeked binding");
                    compare(id, Some(a), Some(b));
                }
                (Some((a, _)), Some((b, _))) if a < b => {
                    let (id, properties) = old.next().expect("peeked binding");
                    compare(id, Some(properties), None);
                }
                (Some(_), None) => {
                    let (id, properties) = old.next().expect("peeked binding");
                    compare(id, Some(properties), None);
                }
                (_, Some(_)) => {
                    let (id, properties) = new.next().expect("peeked binding");
                    compare(id, None, Some(properties));
                }
                (None, None) => break,
            }
        }
    }
    /// Compare only expression trees reachable from visible properties. A changed
    /// value cannot invalidate a frame before the element using it exists.
    pub(crate) fn changed_from(
        &self,
        next: &Self,
        birth: impl FnMut(&str) -> Option<Fraction>,
    ) -> Option<Fraction> {
        let mut earliest = None;
        self.visit_changed_bindings(next, birth, |_, _, time| {
            earliest = earlier(earliest, Some(time))
        });
        earliest
    }
    fn node_changed_from(
        &self,
        next: &Self,
        a: NodeId,
        b: NodeId,
        memo: &mut std::collections::HashMap<(NodeId, NodeId), Option<Fraction>>,
    ) -> Option<Fraction> {
        if let Some(result) = memo.get(&(a, b)) {
            return *result;
        }
        // Adversarial DAG rewiring can produce more pairs than either graph
        // has nodes. Cap diff metadata and conservatively invalidate uncertain
        // bindings rather than letting a patch allocate quadratic memory.
        if memo.len() >= MAX_REACTIVE_NODES {
            return Some(Fraction::from(0));
        }
        // Validated nodes refer only to earlier nodes, so this recursion is bounded
        // by MAX_REACTIVE_DEPTH and memoized once per shared pair of subexpressions.
        let left = &self.nodes[a];
        let right = &next.nodes[b];
        let mut children = |a, b| self.node_changed_from(next, a, b, memo);
        let zero = Some(Fraction::from(0));
        let changed = if left.ty != right.ty {
            zero
        } else {
            match (&left.operation, &right.operation) {
                (Node::Constant(a), Node::Constant(b)) if a == b => None,
                (Node::Cell(a), Node::Cell(b)) => {
                    let a = &self.cells[a];
                    let b = &next.cells[b];
                    if a.ty != b.ty || a.initial != b.initial {
                        zero
                    } else {
                        let common = a
                            .tracks
                            .iter()
                            .zip(&b.tracks)
                            .take_while(|(a, b)| a == b)
                            .count();
                        a.tracks
                            .get(common)
                            .into_iter()
                            .chain(b.tracks.get(common))
                            .map(|track| track.start)
                            .min()
                    }
                }
                (Node::Unary { op: a, value: x }, Node::Unary { op: b, value: y }) if a == b => {
                    children(*x, *y)
                }
                (
                    Node::Binary {
                        op: a,
                        left: x,
                        right: y,
                    },
                    Node::Binary {
                        op: b,
                        left: u,
                        right: v,
                    },
                ) if a == b => earlier(children(*x, *u), children(*y, *v)),
                (
                    Node::Component {
                        vector: a,
                        index: x,
                    },
                    Node::Component {
                        vector: b,
                        index: y,
                    },
                ) if x == y => children(*a, *b),
                (Node::Math { name: a, args: x }, Node::Math { name: b, args: y })
                    if a == b && x.len() == y.len() =>
                {
                    x.iter()
                        .zip(y)
                        .fold(None, |time, (&a, &b)| earlier(time, children(a, b)))
                }
                (Node::Vector { values: a }, Node::Vector { values: b })
                | (Node::List { values: a }, Node::List { values: b })
                    if a.len() == b.len() =>
                {
                    a.iter()
                        .zip(b)
                        .fold(None, |time, (&a, &b)| earlier(time, children(a, b)))
                }
                (Node::Record { fields: a }, Node::Record { fields: b })
                    if a.keys().eq(b.keys()) =>
                {
                    a.values()
                        .zip(b.values())
                        .fold(None, |time, (&a, &b)| earlier(time, children(a, b)))
                }
                (
                    Node::NativeConfig {
                        operation: a,
                        arguments: x,
                    },
                    Node::NativeConfig {
                        operation: b,
                        arguments: y,
                    },
                ) if a == b && x.keys().eq(y.keys()) => x
                    .values()
                    .zip(y.values())
                    .fold(None, |time, (&a, &b)| earlier(time, children(a, b))),
                (Node::Template { parts: a }, Node::Template { parts: b })
                    if a.len() == b.len() =>
                {
                    a.iter().zip(b.iter()).fold(None, |time, (a, b)| {
                        use crate::TemplatePart::{Hole, Text};
                        let changed = match (a, b) {
                            (Text(a), Text(b)) if a == b => None,
                            (
                                Hole {
                                    value: a,
                                    format: x,
                                    conversion: u,
                                },
                                Hole {
                                    value: b,
                                    format: y,
                                    conversion: v,
                                },
                            ) if u == v => {
                                let format = match (x, y) {
                                    (Some(a), Some(b)) => children(*a, *b),
                                    (None, None) => None,
                                    _ => zero,
                                };
                                earlier(children(*a, *b), format)
                            }
                            _ => zero,
                        };
                        earlier(time, changed)
                    })
                }
                _ => zero,
            }
        };
        if memo.len() < MAX_REACTIVE_NODES {
            memo.insert((a, b), changed);
        }
        changed
    }
}
fn earlier(a: Option<Fraction>, b: Option<Fraction>) -> Option<Fraction> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}
fn node_bytes(node: &Node) -> Result<usize, String> {
    Ok(std::mem::size_of::<Expression>()
        + match node {
            Node::Constant(value) => state_value_bytes(&value.ty(), value)?,
            Node::Unary { op, .. } | Node::Binary { op, .. } => op.len(),
            Node::Math { name, args } => name.len() + args.len() * std::mem::size_of::<NodeId>(),
            Node::Vector { values } => values.len() * std::mem::size_of::<NodeId>(),
            Node::List { values } => values.len() * std::mem::size_of::<NodeId>(),
            Node::Record { fields } => fields
                .keys()
                .map(|name| name.len() + std::mem::size_of::<(String, NodeId)>())
                .sum(),
            Node::NativeConfig { arguments, .. } => arguments
                .keys()
                .map(|name| name.len() + std::mem::size_of::<(String, NodeId)>())
                .sum(),
            Node::Template { parts } => {
                std::mem::size_of_val(parts.as_ref())
                    + parts
                        .iter()
                        .map(|part| match part {
                            crate::TemplatePart::Text(text) => text.len(),
                            crate::TemplatePart::Hole { .. } => 0,
                        })
                        .sum::<usize>()
            }
            _ => 0,
        })
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
            Node::NativeConfig {
                operation,
                arguments,
            } => {
                let arguments = arguments
                    .iter()
                    .map(|(name, id)| self.value(*id).map(|value| (name.clone(), value)))
                    .collect::<Result<imbl::OrdMap<_, _>, _>>()?;
                operation.execute(&arguments)?
            }
            Node::Constant(value) => value.clone(),
            Node::List { values } => {
                let Type::List(ty, _) = &graph.nodes[id].ty else {
                    unreachable!("validated retained list")
                };
                Value::List(crate::List::typed(
                    ty.as_ref().clone(),
                    values
                        .iter()
                        .map(|id| self.value(*id))
                        .collect::<Result<Vec<_>, _>>()?,
                ))
            }
            Node::Record { fields } => Value::Record(
                fields
                    .iter()
                    .map(|(name, id)| self.value(*id).map(|value| (name.clone(), value)))
                    .collect::<Result<_, _>>()?,
            ),
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
            Node::Template { parts } => Value::String(
                crate::template::render_with(
                    parts,
                    |id| self.value(*id),
                    math::format::MAX_OUTPUT_BYTES,
                )?
                .into(),
            ),
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
fn state_schema_bytes(ty: &Type) -> Result<usize, String> {
    let mut remaining = MAX_REACTIVE_BYTES;
    crate::type_budget::charge(ty, &mut remaining)?;
    if !ty.is_concrete() || ty.contains_function() {
        return Err("retained state requires a complete static schema; use set_typed_cell for empty options/lists and union types".into());
    }
    Ok(MAX_REACTIVE_BYTES - remaining)
}
fn state_value_bytes(ty: &Type, value: &Value) -> Result<usize, String> {
    let mut remaining = MAX_REACTIVE_BYTES;
    let mut values = MAX_REACTIVE_BYTES / std::mem::size_of::<Value>();
    crate::value_budget::charge(value, &mut remaining, &mut values, MAX_REACTIVE_DEPTH)?;
    crate::validate_value(ty, value)?;
    Ok(MAX_REACTIVE_BYTES - remaining)
}
fn interpolated_type(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Numeric(_)
            | Type::RefinedNumeric(_)
            | Type::Fraction
            | Type::Color
            | Type::Vector(2..=4, Some(_))
    )
}
fn leaf_type(value: &Value) -> Result<Type, String> {
    match value {
        Value::Number(number) if math::finite(&number.value) => {}
        Value::Fraction(value) if math::finite(value) => {}
        Value::Bool(_) => {}
        Value::String(text) if text.len() <= math::format::MAX_OUTPUT_BYTES => {}
        Value::Font(font) => {
            font.validate()?;
        }
        Value::Code(_)
        | Value::List(_)
        | Value::Record(_)
        | Value::Some(_)
        | Value::None
        | Value::Variant { .. } => {
            let mut bytes = MAX_REACTIVE_BYTES;
            let mut items = MAX_REACTIVE_NODES;
            crate::value_budget::charge(value, &mut bytes, &mut items, MAX_REACTIVE_DEPTH)?;
            crate::validate_value(&value.ty(), value)?;
        }
        Value::Color(color)
            if color
                .0
                .iter()
                .all(|v| math::finite(v) && *v >= Fraction::from(0) && *v <= Fraction::from(1)) => {
        }
        Value::Vec2(_) | Value::Vec3(_) | Value::Vec4(_)
            if value.vector()?.0.iter().all(|v| v.is_finite()) => {}
        _ => {
            return Err(format!(
                "a value of type {} cannot be retained as a native animated argument; use finite scalar, font, code, enum or structured configuration values",
                value.ty()
            ));
        }
    }
    Ok(value.ty())
}
fn property_type(property: Property, ty: &Type) -> Result<(), String> {
    if let Some(expected) = crate::code::view::input_type(property) {
        return if expected.accepts(ty) {
            Ok(())
        } else {
            Err(format!(
                "retained {property:?} requires {expected}, found {ty}"
            ))
        };
    }
    let valid = match property {
        Property::Position | Property::Scroll => {
            matches!(ty, Type::Vector(2, Some(Unit::Length | Unit::Percent)))
        }
        Property::Rotation => math::operations::scalar_unit(ty) == Some(Unit::Angle),
        Property::Scale | Property::StretchX | Property::Opacity | Property::Reveal => {
            math::operations::scalar_unit(ty) == Some(Unit::Number)
        }
        Property::Radius => math::operations::scalar_unit(ty) == Some(Unit::Length),
        Property::Width | Property::Height => matches!(
            math::operations::scalar_unit(ty),
            Some(Unit::Length | Unit::Percent)
        ),
        Property::Fill => crate::Paint::ty().accepts(ty),
        Property::Code
        | Property::CodeLanguage
        | Property::CodeTheme
        | Property::CodeSpans
        | Property::CodeLineHeight
        | Property::CodeTabSize
        | Property::CodeLineNumbers
        | Property::CodeScopeIndicators => unreachable!("CodeView properties validated above"),
        Property::Overflow | Property::OverflowX | Property::OverflowY => {
            crate::Overflow::ty().accepts(ty)
        }
        Property::Text => Type::String.accepts(ty),
        Property::WhiteSpace => crate::text::WhiteSpace::ty().accepts(ty),
        Property::OverflowWrap => crate::text::OverflowWrap::ty().accepts(ty),
        Property::Font => *ty == Type::Font,
        Property::FontSize => math::operations::scalar_unit(ty) == Some(Unit::Length),
        Property::XAlign | Property::YAlign => {
            use crate::NativeEnum;
            *ty == crate::Align::ty()
        }
    };
    if valid {
        Ok(())
    } else {
        Err(format!("retained {property:?} has incompatible type {ty}"))
    }
}
