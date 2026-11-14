//! Persistent lexical cells and memoized, dynamically tracked signal values.
use super::*;
pub(super) type Id = u64;
#[derive(Clone, serde::Serialize)]
pub(super) enum Binding {
    Value(Value),
    Reactive(Id),
    Deferred(Id),
    Module(String),
}
impl Binding {
    pub(super) fn is_dynamic(&self) -> bool {
        match self {
            Self::Reactive(_) | Self::Deferred(_) | Self::Module(_) => true,
            Self::Value(value) => value.ty().contains_signal(),
        }
    }
}
impl From<Value> for Binding {
    fn from(value: Value) -> Self {
        Self::Value(value)
    }
}
#[derive(Clone, serde::Serialize)]
pub(super) struct Cell {
    pub value: Value,
    pub ty: Arc<Type>,
    pub name: String,
    pub module: String,
    pub span: Span,
    pub sampled_at: Option<Fraction>,
    pub creation_background_depth: usize,
}
#[derive(Clone, serde::Serialize)]
pub(super) struct Signal {
    pub frozen_at: Option<Fraction>,
    pub expression: Arc<Expr>,
    pub scope: Env,
    pub module: String,
    pub name: String,
    pub span: Span,
    pub ty: Arc<Type>,
    pub types: BTreeMap<String, Type>,
    pub identity: Vec<String>,
    #[serde(skip)]
    pub source_calls: imbl::Vector<crate::SourceLocation>,
    pub cached: Option<Value>,
    pub dependencies: imbl::OrdSet<Id>,
    pub dirty: bool,
    pub temporal: bool,
    pub sampled_at: Option<Fraction>,
}
#[derive(Clone, serde::Serialize)]
pub(super) enum Node {
    Cell(Cell),
    Signal(Signal),
}
#[derive(Clone, Default, serde::Serialize)]
pub(super) struct Store {
    pub next: Id,
    pub revision: u64,
    pub nodes: imbl::OrdMap<Id, Node>,
    dependents: imbl::OrdMap<Id, imbl::OrdSet<Id>>,
    edges: usize,
    retained: imbl::OrdSet<Id>,
    pub lowered: imbl::OrdMap<Id, usize>,
}
pub(super) struct Frame {
    pub id: Id,
    pub dependencies: imbl::OrdSet<Id>,
    pub temporal: bool,
}
impl Store {
    pub fn cached(
        &self,
        id: Id,
        time: Fraction,
        graph: &sanim_core::reactive::ReactiveGraph,
    ) -> Option<&Value> {
        let clean_cell = |id: &Id| matches!(self.nodes.get(id),Some(Node::Cell(cell)) if !graph.has_cell(*id)||cell.sampled_at==Some(time));
        match self.nodes.get(&id)? {
            Node::Cell(cell) if clean_cell(&id) => Some(&cell.value),
            Node::Signal(signal)
                if !signal.dirty
                    && (!signal.temporal || signal.sampled_at == Some(time))
                    && signal.dependencies.iter().all(clean_cell) =>
            {
                signal.cached.as_ref()
            }
            _ => None,
        }
    }
    pub fn retain(&mut self, id: Id) {
        self.touch();
        let mut pending = vec![id];
        while let Some(id) = pending.pop() {
            if self.retained.contains(&id) {
                continue;
            }
            self.retained.insert(id);
            if let Some(Node::Signal(signal)) = self.nodes.get(&id) {
                pending.extend(signal.dependencies.iter().copied());
                for binding in signal.scope.values() {
                    match binding {
                        Binding::Reactive(id) | Binding::Deferred(id) => pending.push(*id),
                        Binding::Value(Value::Signal(signal)) => pending.push(signal.id),
                        _ => {}
                    }
                }
            }
        }
    }
    pub fn touch(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("reactive revision counter overflow");
    }
    pub fn overhead(&self) -> usize {
        self.nodes
            .len()
            .saturating_mul(std::mem::size_of::<Node>() + std::mem::size_of::<Id>())
            + self.edges.saturating_mul(std::mem::size_of::<Id>())
            + self
                .lowered
                .len()
                .saturating_mul(std::mem::size_of::<Id>() + std::mem::size_of::<usize>())
    }
    fn link(&mut self, id: Id, dependencies: imbl::OrdSet<Id>) {
        let previous = match self.nodes.get(&id) {
            Some(Node::Signal(signal)) => signal.dependencies.clone(),
            _ => return,
        };
        for cell in previous.iter() {
            if !dependencies.contains(cell) {
                self.unlink(*cell, id);
            }
        }
        for cell in dependencies.iter() {
            if !previous.contains(cell) {
                self.dependents.entry(*cell).or_default().insert(id);
                self.edges += 1;
            }
        }
        if let Some(Node::Signal(signal)) = self.nodes.get_mut(&id) {
            signal.dependencies = dependencies;
        }
    }
    fn unlink(&mut self, cell: Id, signal: Id) {
        if let Some(ids) = self.dependents.get_mut(&cell) {
            if ids.contains(&signal) {
                ids.remove(&signal);
                self.edges -= 1;
            }
            if ids.is_empty() {
                self.dependents.remove(&cell);
            }
        }
    }
    pub(super) fn remove_since(&mut self, first: Id) {
        let dead = self
            .nodes
            .range(first..)
            .filter_map(|(id, _)| (!self.retained.contains(id)).then_some(*id))
            .collect::<Vec<_>>();
        if !dead.is_empty() {
            self.touch();
        }
        for id in dead {
            if let Some(Node::Signal(signal)) = self.nodes.remove(&id) {
                for cell in &signal.dependencies {
                    self.unlink(*cell, id);
                }
            }
            if let Some(ids) = self.dependents.remove(&id) {
                self.edges -= ids.len();
            }
            self.lowered.remove(&id);
        }
    }
}
impl Runtime<'_, '_> {
    pub(super) fn binding_value(
        &mut self,
        binding: &Binding,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        match binding {
            Binding::Value(value) => self.sample_signal(value.clone(), span),
            Binding::Reactive(id) | Binding::Deferred(id) => self.read_reactive(*id, span),
            Binding::Module(module) => {
                let scope = self
                    .globals
                    .get(module)
                    .cloned()
                    .ok_or_else(|| self.error("S301", "imported module has no values", span))?;
                self.materialize(&scope, span).map(Value::Record)
            }
        }
    }
    pub(super) fn materialize(&mut self, env: &Env, span: Span) -> Result<Values, Diagnostic> {
        let mut values = Values::new();
        for (name, binding) in env {
            values.insert(name.clone(), self.binding_value(binding, span)?);
        }
        Ok(values)
    }
    pub(super) fn read_reactive(&mut self, id: Id, span: Span) -> Result<Value, Diagnostic> {
        let node = self.reactive.nodes.get(&id).cloned().ok_or_else(|| {
            self.error("S140", "reactive binding escaped its lexical scope", span)
        })?;
        match node {
            Node::Cell(_) => {
                self.refresh_cell(id, span)?;
                let Some(Node::Cell(cell)) = self.reactive.nodes.get(&id) else {
                    unreachable!("live cell")
                };
                if let Some(frame) = self.signal_frames.last_mut() {
                    frame.dependencies.insert(id);
                }
                Ok(cell.value.clone())
            }
            Node::Signal(signal) => {
                for dependency in &signal.dependencies {
                    self.refresh_cell(*dependency, span)?;
                }
                let Some(Node::Signal(signal)) = self.reactive.nodes.get(&id).cloned() else {
                    unreachable!("live signal")
                };
                if self.signal_frames.iter().any(|frame| frame.id == id) {
                    let chain = self
                        .signal_frames
                        .iter()
                        .filter_map(|frame| match self.reactive.nodes.get(&frame.id) {
                            Some(Node::Signal(signal)) => Some(signal.name.as_str()),
                            _ => None,
                        })
                        .chain(std::iter::once(signal.name.as_str()))
                        .collect::<Vec<_>>()
                        .join(" -> ");
                    return Err(self.error("S141",format!("reactive dependency cycle: {chain}; remove the signal's dependency on itself, including through helper calls"),span));
                }
                if !signal.dirty
                    && (!signal.temporal || signal.sampled_at == Some(self.cursor))
                    && let Some(value) = &signal.cached
                {
                    self.statistics.signal_cache_hits += 1;
                    if let Some(frame) = self.signal_frames.last_mut() {
                        frame
                            .dependencies
                            .extend(signal.dependencies.iter().copied());
                        frame.temporal |= signal.temporal;
                    }
                    return Ok(value.clone());
                }
                if self.signal_frames.len() >= self.limits.max_call_depth {
                    return Err(self.error(
                        "S303",
                        "signal dependency depth exceeds the bounded call-depth limit",
                        span,
                    ));
                }
                self.signal_frames.push(Frame {
                    id,
                    dependencies: Default::default(),
                    temporal: false,
                });
                let previous_module = std::mem::replace(&mut self.module, signal.module.clone());
                let previous_types =
                    std::mem::replace(&mut self.type_bindings, signal.types.clone());
                let previous_identity =
                    std::mem::replace(&mut self.identity, signal.identity.clone());
                let previous_calls =
                    std::mem::replace(&mut self.source_calls, signal.source_calls.clone());
                let previous_name = self
                    .named_constructor
                    .replace((signal.expression.span.start, signal.name.clone()));
                let previous_time = self.cursor;
                if let Some(time) = signal.frozen_at {
                    self.cursor = time;
                }
                let result = self.expression(&signal.expression, &signal.scope);
                self.cursor = previous_time;
                self.module = previous_module;
                self.type_bindings = previous_types;
                self.identity = previous_identity;
                self.source_calls = previous_calls;
                self.named_constructor = previous_name;
                let mut frame = self.signal_frames.pop().expect("active signal frame");
                let value = result?
                    .with_type(&signal.ty)
                    .map_err(|e| self.error("S140", e, signal.span))?;
                validate_value(&signal.ty, &value).map_err(|e| {
                    self.error(
                        "S140",
                        format!("signal '{}': {e}", signal.name),
                        signal.span,
                    )
                })?;
                frame.dependencies = frame
                    .dependencies
                    .into_iter()
                    .filter(|id| matches!(self.reactive.nodes.get(id), Some(Node::Cell(_))))
                    .collect();
                self.reactive.link(id, frame.dependencies.clone());
                self.retain_binding_value(&value, signal.span)?;
                if let Some(Node::Signal(current)) = self.reactive.nodes.get_mut(&id) {
                    current.cached = Some(value.clone());
                    current.dirty = false;
                    current.temporal = frame.temporal;
                    current.sampled_at = Some(self.cursor);
                }
                if let Some(parent) = self.signal_frames.last_mut() {
                    parent.dependencies.extend(frame.dependencies);
                    parent.temporal |= frame.temporal;
                }
                self.reactive.touch();
                if signal.frozen_at.is_some() {
                    self.statistics.forced_values += 1;
                } else {
                    self.statistics.signal_evaluations += 1;
                }
                Ok(value)
            }
        }
    }
    pub(super) fn declare_reactive(
        &mut self,
        name: &str,
        kind: BindingKind,
        expr: &Expr,
        env: &Env,
        span: Span,
    ) -> Result<Binding, Diagnostic> {
        if self.reactive.nodes.len() >= self.limits.data.max_values {
            return Err(self.error("S303","live reactive bindings exceed the configured data-value limit; reduce retained signals/cells or loop output",span));
        }
        let ty = self.binding_type(span)?;
        if kind==BindingKind::Sig && let ExprKind::Call{callee,..}=&expr.kind
            && self.loaded.iter().find(|module|module.id==self.module).and_then(|module|crate::effects::lookup(callee,&module.checked.exports)).is_some_and(|ty|matches!(ty,Type::Function(signature) if matches!(signature.result.as_ref(),Type::Signal(_)))) {
            let Value::Signal(signal)=self.capture_signal(expr,env,&ty)? else{unreachable!("checked signal factory")};
            return Ok(Binding::Reactive(signal.id));
        }
        let node = if kind == BindingKind::Mut {
            let value = self
                .expression(expr, env)?
                .with_type(&ty)
                .map_err(|e| self.error("S140", e, span))?;
            validate_value(&ty, &value)
                .map_err(|e| self.error("S140", format!("mutable binding '{name}': {e}"), span))?;
            self.retain_binding_value(&value, span)?;
            Node::Cell(Cell {
                value,
                ty: Arc::new(ty),
                name: name.into(),
                module: self.module.clone(),
                span,
                sampled_at: None,
                creation_background_depth: self.background_depth,
            })
        } else {
            let mut names = BTreeSet::new();
            dependencies(expr, &mut names);
            let scope = env
                .iter()
                .filter(|(name, _)| names.contains(*name))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect();
            self.charge_signal_metadata(name, expr, &scope, &ty, span)?;
            Node::Signal(Signal {
                frozen_at: None,
                expression: Arc::new(expr.clone()),
                scope,
                module: self.module.clone(),
                name: name.into(),
                span,
                ty: Arc::new(ty),
                types: self.type_bindings.clone(),
                identity: self.identity.clone(),
                source_calls: self.source_calls.clone(),
                cached: None,
                dependencies: Default::default(),
                dirty: true,
                temporal: false,
                sampled_at: None,
            })
        };
        let id = self.reactive.next;
        self.reactive.next = id.checked_add(1).ok_or_else(|| {
            self.error("S303", "reactive binding identity counter overflow", span)
        })?;
        self.reactive.nodes.insert(id, node);
        self.reactive.touch();
        Ok(Binding::Reactive(id))
    }
    pub(super) fn assign(
        &mut self,
        name: &str,
        expr: &Expr,
        env: &Env,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let binding = env.get(name).cloned().ok_or_else(|| {
            self.error("S120", format!("unknown assignment target '{name}'"), span)
        })?;
        let Binding::Reactive(id) = binding else {
            return Err(self.error(
                "S140",
                format!("'{name}' is immutable; declare it with mut to allow assignment"),
                span,
            ));
        };
        self.refresh_cell(id, span)?;
        let Some(Node::Cell(cell)) = self.reactive.nodes.get(&id).cloned() else {
            return Err(self.error(
                "S140",
                format!("'{name}' is a read-only signal; assign to its mut inputs instead"),
                span,
            ));
        };
        let list = matches!(cell.value, Value::List(_));
        let timed = self.animation.reactive.has_cell(id)
            || self.animation.list_views.has_cell(id)
            || self.background_depth > 0 && cell.creation_background_depth != self.background_depth;
        if timed {
            if list {
                self.ensure_list_cell(id, span)?;
            } else if !self.animation.reactive.has_cell(id) {
                self.ensure_animated_cell(id, span)?;
            }
        }
        // Fully evaluate and validate against the old store before committing the write.
        let value = self
            .expression(expr, env)?
            .with_type(&cell.ty)
            .map_err(|e| self.error("S140", e, span))?;
        self.refresh_cell(id, span)?;
        let current = match self.reactive.nodes.get(&id) {
            Some(Node::Cell(current)) => current.value.clone(),
            _ => unreachable!("live assignment target"),
        };
        let delta = self.take_list_edit(name, expr, &current, &value, &cell.ty);
        if delta.is_none() {
            validate_value(&cell.ty, &value)
                .map_err(|e| self.error("S140", format!("cannot assign '{name}': {e}"), span))?;
        }
        let (start, easing) = self.assignment_timing(expr);
        let unchanged = match (self.reactive.nodes.get(&id), &value, &delta) {
            (Some(Node::Cell(current)), Value::List(value), Some(delta))
                if delta.cow_bytes == 0 =>
            {
                matches!(&current.value, Value::List(current) if current.values.ptr_eq(&value.values) || current == value)
            }
            (_, _, Some(_)) => false,
            (Some(Node::Cell(current)), value, None) => current.value == *value,
            _ => false,
        };
        if unchanged {
            if self.background_depth > 0 && timed {
                // An explicit future event may override a main-thread write inserted earlier later.
                if list {
                    self.write_list_assignment(
                        id,
                        &value,
                        start,
                        delta.as_ref().map(|delta| &delta.edit),
                        easing,
                        span,
                    )?;
                } else {
                    self.animation
                        .reactive
                        .write_cell(id, self.cursor, value)
                        .map_err(|error| self.error("S140", error, span))?;
                }
                if let Some(Node::Cell(cell)) = self.reactive.nodes.get_mut(&id) {
                    cell.sampled_at = Some(self.cursor);
                }
                self.reactive.touch();
            }
            self.statistics.unchanged_writes += 1;
            return Ok(());
        }
        if let Some(delta) = &delta {
            self.retain_list_edit(delta, span)?;
        } else {
            self.retain_binding_value(&value, span)?;
        }
        if list && (timed || self.cursor > start) {
            self.write_list_assignment(
                id,
                &value,
                start,
                delta.as_ref().map(|delta| &delta.edit),
                easing,
                span,
            )?;
        } else if self.cursor > start {
            self.ensure_animated_cell(id, span)?;
            self.animation
                .reactive
                .animate_cell(id, start, self.cursor, value.clone(), easing)
                .map_err(|error| self.error("S140", error, span))?;
        } else if self.animation.reactive.has_cell(id) {
            self.animation
                .reactive
                .write_cell(id, self.cursor, value.clone())
                .map_err(|e| self.error("S140", e, span))?;
        }
        let Some(Node::Cell(current)) = self.reactive.nodes.get_mut(&id) else {
            unreachable!("validated mutable cell")
        };
        current.value = value;
        current.sampled_at = Some(self.cursor);
        self.invalidate_cell(id);
        self.reactive.touch();
        self.statistics.mutable_writes += 1;
        Ok(())
    }
    pub(super) fn binding_type(&self, span: Span) -> Result<Type, Diagnostic> {
        self.loaded
            .iter()
            .find(|module| module.id == self.module)
            .and_then(|module| module.checked.binding_types.get(&span.start))
            .map(|ty| ty.substitute(&self.type_bindings))
            .ok_or_else(|| self.error("S140", "binding is missing its checked static type", span))
    }
    pub(super) fn refresh_cell(&mut self, id: Id, span: Span) -> Result<(), Diagnostic> {
        if !self.animation.reactive.has_cell(id) && !self.animation.list_views.has_cell(id) {
            return Ok(());
        }
        let Some(Node::Cell(cell)) = self.reactive.nodes.get(&id) else {
            return Ok(());
        };
        if cell.sampled_at == Some(self.cursor) {
            return Ok(());
        }
        let value = if self.animation.list_views.has_cell(id) {
            Value::List(
                self.animation
                    .sample_list(id, self.cursor)
                    .map_err(|error| self.error("S140", error, span))?
                    .expect("registered list channel"),
            )
        } else {
            self.animation
                .reactive
                .sample_cell(id, self.cursor)
                .map_err(|error| self.error("S140", error, span))?
        };
        let Some(Node::Cell(cell)) = self.reactive.nodes.get_mut(&id) else {
            unreachable!("live cell")
        };
        let changed = cell.value != value;
        cell.value = value;
        cell.sampled_at = Some(self.cursor);
        self.reactive.touch();
        if changed {
            self.invalidate_cell(id);
        }
        Ok(())
    }
    fn invalidate_cell(&mut self, id: Id) {
        if let Some(dependents) = self.reactive.dependents.get(&id).cloned() {
            for signal in dependents {
                if let Some(Node::Signal(signal)) = self.reactive.nodes.get_mut(&signal)
                    && !signal.dirty
                {
                    signal.dirty = true;
                    self.statistics.signal_invalidations += 1;
                }
            }
        }
    }
    pub(super) fn retain_binding_value(
        &mut self,
        value: &Value,
        span: Span,
    ) -> Result<(), Diagnostic> {
        match value {
            Value::Signal(signal) => {
                self.reactive.retain(signal.id);
            }
            Value::DataFrame(frame) => {
                return frame
                    .retain(
                        &mut self.retained_frames,
                        &mut self.retained_bytes,
                        &mut self.retained_items,
                    )
                    .map_err(|message| self.error("S303", message, span));
            }
            Value::Series(series) => {
                series
                    .frame
                    .retain(
                        &mut self.retained_frames,
                        &mut self.retained_bytes,
                        &mut self.retained_items,
                    )
                    .map_err(|message| self.error("S303", message, span))?;
                self.retained_bytes = self
                    .retained_bytes
                    .checked_sub(series.expression_bytes)
                    .ok_or_else(|| {
                        self.error("S303", "retained Series byte budget exceeded", span)
                    })?;
                self.retained_items =
                    self.retained_items
                        .checked_sub(series.nodes)
                        .ok_or_else(|| {
                            self.error("S303", "retained Series value budget exceeded", span)
                        })?;
                return Ok(());
            }
            _ => {}
        }
        sanim_core::value_budget::charge_with_budget(
            value,
            &mut self.retained_bytes,
            &mut self.retained_items,
            self.limits.data.max_depth,
            &mut self.retained_fonts,
        )
        .map_err(|e| self.error("S303", format!("preparation binding budget: {e}"), span))
    }
    pub(super) fn scoped_block(
        &mut self,
        body: &[Statement],
        env: &mut Env,
    ) -> Result<Option<Value>, Diagnostic> {
        let first = self.reactive.next;
        let result = self.block(body, env);
        self.reactive.remove_since(first);
        result
    }
}

impl Runtime<'_, '_> {
    pub(super) fn function_mutates(&self, function: &Function) -> bool {
        self.function_signature(function)
            .is_some_and(|signature| signature.mutates)
    }
    pub(super) fn instance_site(&self, span: Span) -> String {
        self.instance_paths
            .get(&self.module)
            .and_then(|paths| paths.get(&span.start))
            .map_or_else(
                || format!("{}::expression", self.module),
                |path| format!("{}::{path}", self.module),
            )
    }
}
pub(super) fn instance_paths(module: &Module) -> BTreeMap<usize, String> {
    fn expression(expr: &Expr, path: &str, out: &mut BTreeMap<usize, String>) {
        let mut index = 0;
        expr.visit(&mut |expr| {
            if matches!(expr.kind, ExprKind::Call { .. }) {
                out.insert(expr.span.start, format!("{path}.call{index}"));
                index += 1;
            }
        });
    }
    fn block(body: &[Statement], parent: &str, out: &mut BTreeMap<usize, String>) {
        for (index, statement) in body.iter().enumerate() {
            let path = format!("{parent}.{index}");
            match &statement.kind {
                StatementKind::Bind { value, .. }
                | StatementKind::Assign { value, .. }
                | StatementKind::Expr(value)
                | StatementKind::Return(value) => expression(value, &path, out),
                StatementKind::Animate {
                    value,
                    duration,
                    easing,
                    ..
                } => {
                    expression(value, &format!("{path}.target"), out);
                    expression(duration, &format!("{path}.duration"), out);
                    if let Some(easing) = easing {
                        expression(easing, &format!("{path}.ease"), out);
                    }
                }
                StatementKind::Function {
                    name,
                    parameters,
                    body,
                    ..
                } => {
                    let path = format!("fn.{name}");
                    for parameter in parameters {
                        let name = &parameter.name;
                        let value = &parameter.default;
                        if let Some(value) = value {
                            expression(value, &format!("{path}.{name}"), out);
                        }
                    }
                    block(body, &path, out);
                }
                StatementKind::If { condition, yes, no } => {
                    expression(condition, &path, out);
                    block(yes, &format!("{path}.yes"), out);
                    block(no, &format!("{path}.no"), out);
                }
                StatementKind::For { iterable, body, .. } => {
                    out.insert(statement.span.start, path.clone());
                    expression(iterable, &format!("{path}.items"), out);
                    block(body, &path, out);
                }
                StatementKind::Repeat {
                    count,
                    period,
                    body,
                } => {
                    out.insert(statement.span.start, path.clone());
                    expression(count, &format!("{path}.count"), out);
                    if let Some(period) = period {
                        expression(period, &format!("{path}.period"), out);
                    }
                    block(body, &path, out);
                }
                StatementKind::Match { value, cases } => {
                    expression(value, &path, out);
                    for (index, case) in cases.iter().enumerate() {
                        block(&case.body, &format!("{path}.case{index}"), out);
                    }
                }
                StatementKind::Background { body } => block(body, &path, out),
                _ => {}
            }
        }
    }
    let mut out = BTreeMap::new();
    block(&module.statements, "root", &mut out);
    out
}

impl Runtime<'_, '_> {
    pub(super) fn expression_mutates(&self, expr: &Expr) -> bool {
        let Some(module) = self.loaded.iter().find(|module| module.id == self.module) else {
            return true;
        };
        let mut mutates = false;
        expr.visit_while(&mut |expr| {
            if self.observed_signal(expr).is_some() {
                return false;
            }
            if let ExprKind::Call { callee, args, .. } = &expr.kind {
                mutates |= crate::effects::lookup(callee, &module.checked.exports)
                    .is_some_and(|ty| ty.signatures().iter().any(|signature| signature.mutates))
                    || crate::timing::builtin_call(callee, args, &module.checked.native_methods)
                        .is_some_and(|call| {
                            crate::list_functions::callback_type(&call, &module.checked.exports)
                                .is_some_and(|ty| {
                                    ty.signatures().iter().any(|signature| signature.mutates)
                                })
                        });
            }
            !mutates
        });
        mutates
    }
    pub(super) fn statement_stateful(&self, statement: &Statement) -> bool {
        match &statement.kind {
            StatementKind::Bind { kind, value, .. } => {
                *kind != BindingKind::Let || self.expression_mutates(value)
            }
            StatementKind::Assign { .. }
            | StatementKind::Animate { .. }
            | StatementKind::Background { .. } => true,
            StatementKind::If { condition, yes, no } => {
                self.expression_mutates(condition)
                    || yes
                        .iter()
                        .chain(no)
                        .any(|statement| self.statement_stateful(statement))
            }
            StatementKind::For { iterable, body, .. } => {
                self.expression_mutates(iterable)
                    || body
                        .iter()
                        .any(|statement| self.statement_stateful(statement))
            }
            StatementKind::Repeat {
                count,
                period,
                body,
            } => {
                self.expression_mutates(count)
                    || period
                        .as_ref()
                        .is_some_and(|expr| self.expression_mutates(expr))
                    || body
                        .iter()
                        .any(|statement| self.statement_stateful(statement))
            }
            StatementKind::Match { value, cases } => {
                self.expression_mutates(value)
                    || cases.iter().any(|case| {
                        case.body
                            .iter()
                            .any(|statement| self.statement_stateful(statement))
                    })
            }
            StatementKind::Expr(expr) | StatementKind::Return(expr) => {
                self.expression_mutates(expr)
            }
            _ => false,
        }
    }
}

impl Runtime<'_, '_> {
    pub(super) fn expression_schedules(&self, expr: &Expr) -> bool {
        let Some(module) = self.loaded.iter().find(|module| module.id == self.module) else {
            return true;
        };
        let mut schedules = false;
        expr.visit_while(&mut |expr| {
            if self.observed_signal(expr).is_some() {
                return false;
            }
            if let ExprKind::Call { callee, args, .. } = &expr.kind {
                schedules |= crate::timing::builtin_call(
                    callee,
                    args,
                    &self
                        .loaded
                        .iter()
                        .find(|module| module.id == self.module)
                        .expect("checked module")
                        .checked
                        .native_methods,
                )
                .is_some_and(|call| {
                    call.schedules()
                        || crate::list_functions::callback_type(&call, &module.checked.exports)
                            .is_some_and(|ty| {
                                ty.signatures().iter().any(|signature| signature.schedules)
                            })
                });
                if let Some(ty) = crate::effects::lookup(callee, &module.checked.exports) {
                    schedules |= ty.signatures().iter().any(|signature| {
                        signature.schedules
                            || !signature
                                .parameters
                                .iter()
                                .any(|parameter| parameter.name == "duration")
                                && crate::timing::envelope(args)
                                    .is_some_and(|duration| !crate::timing::known_zero(duration))
                    });
                }
            }
            !schedules
        });
        schedules
    }
}
