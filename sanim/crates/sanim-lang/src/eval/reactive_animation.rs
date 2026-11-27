//! Lower explicit signal properties once to the renderer's bounded typed DAG.
use super::*;
use reactive::{Binding, Id, Node as Stored};
use sanim_core::reactive::{Node, NodeId};
impl Runtime<'_, '_> {
    pub(super) fn infer_constructor_id(
        &self,
        name: &str,
        args: &[(Option<String>, Expr)],
        values: &mut Vec<(Option<String>, Value)>,
        span: Span,
    ) {
        let Some((start, binding)) = &self.named_constructor else {
            return;
        };
        if *start != span.start {
            return;
        }
        let Some(builtin) =
            prelude::builtin(name).filter(|builtin| builtin.result == Type::Element)
        else {
            return;
        };
        let Some(index) = builtin
            .parameters
            .iter()
            .position(|parameter| parameter.name == "id")
        else {
            return;
        };
        if args.iter().any(|(name, _)| name.as_deref() == Some("id"))
            || args.iter().filter(|(name, _)| name.is_none()).count() > index
        {
            return;
        }
        let binding = if self
            .loaded
            .last()
            .is_some_and(|root| root.id == self.module)
        {
            binding.clone()
        } else {
            format!("{}::{binding}", self.module)
        };
        let id = if self.identity.is_empty() {
            binding
        } else {
            format!("{binding}@{}", self.identity.join("/"))
        };
        values.push((Some("id".into()), Value::String(id.into())));
    }
    pub(super) fn ensure_animated_cell(&mut self, id: Id, span: Span) -> Result<(), Diagnostic> {
        self.refresh_cell(id, span)?;
        if !self.animation.reactive.has_cell(id) {
            let Some(Stored::Cell(cell)) = self.reactive.nodes.get(&id) else {
                return Err(self.error("S140", "animation target must be a mutable cell", span));
            };
            self.animation
                .reactive
                .set_typed_cell(id, cell.ty.as_ref().clone(), cell.value.clone())
                .map_err(|e| {
                    self.error(
                        "S140",
                        format!("timed writes to '{}': {e}", cell.name),
                        span,
                    )
                })?;
        }
        self.reactive.retain(id);
        Ok(())
    }
    pub(super) fn cell_animation(
        &mut self,
        builtin: &prelude::Builtin,
        args: &[(Option<String>, Expr)],
        env: &Env,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        let mut positional = 0;
        let mut target = None;
        let mut values = Values::new();
        for (label, expression) in args {
            let name = if let Some(label) = label {
                label.as_str()
            } else {
                let name = builtin
                    .parameters
                    .get(positional)
                    .expect("checked animation argument")
                    .name;
                positional += 1;
                name
            };
            if name == builtin.parameters[0].name {
                let ExprKind::Name(name) = &expression.kind else {
                    return Err(self.error(
                        "S140",
                        "animate target must name a mutable binding",
                        expression.span,
                    ));
                };
                let Some(Binding::Reactive(id)) = env.get(name) else {
                    return Err(self.error(
                        "S140",
                        format!("animate target '{name}' must be declared with mut"),
                        expression.span,
                    ));
                };
                self.ensure_animated_cell(*id, span)?;
                target = Some(*id);
            } else {
                values.insert(name.to_owned(), self.expression(expression, env)?);
            }
        }
        let id = target.expect("checked mutable animation target");
        let curve = match values.remove("curve") {
            Some(Value::Curve(curve)) => Some(curve),
            Some(_) => {
                return Err(self.error(
                    "S140",
                    "animate curve must be a typed Curve(keys=...)",
                    span,
                ));
            }
            None => None,
        };
        let duration = if let Some(curve)=&curve {
            let natural=curve.duration().ok_or_else(||self.error("S140","animate(curve=...) requires Keyframe timestamps, not geometric points",span))?;
            if values.contains_key("ease"){return Err(self.error("S140","keyframed curves define their own time handles; remove ease= or adjust Keyframe handles",span));}
            if values.contains_key("duration"){builtin.timing.value(values.get("duration"))}else{Ok(natural)}
        }else{builtin.timing.value(values.get("duration"))}
            .map_err(|message| self.error("S304", message, span))?;
        let easing = crate::timing::easing(values.get("ease"))
            .map_err(|message| self.error("S304", message, span))?;
        let to = if let Some(curve) = &curve {
            curve
                .at(1.into())
                .map_err(|e| self.error("S140", e, span))?
        } else {
            values
                .remove("to")
                .ok_or_else(|| self.error("S140", "animate requires to=... or curve=...", span))?
        };
        let mut value = crate::animation_plan::cell(id, to, duration, easing, curve);
        if let Value::Animation(plan) = &mut value {
            std::sync::Arc::make_mut(plan).origin = Some(sanim_core::plan::Origin {
                checkpoint: self.root_statement,
                location: self.source_location(span),
                binding: None,
            });
        }
        Ok(value)
    }
    fn signal_in(&self, expr: &Expr, env: &Env) -> bool {
        fn binding<'b>(
            runtime: &'b Runtime<'_, '_>,
            expr: &Expr,
            env: &'b Env,
        ) -> Option<&'b Binding> {
            match &expr.kind {
                ExprKind::Name(name) => env.get(name),
                ExprKind::Member(base, name) => {
                    if let Some(Binding::Module(module)) = binding(runtime, base, env) {
                        runtime
                            .globals
                            .get(module)
                            .and_then(|scope| scope.get(name))
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
        fn node(
            runtime: &Runtime<'_, '_>,
            expr: &Expr,
            env: &Env,
            visited: &mut BTreeSet<String>,
        ) -> bool {
            if matches!(
                binding(runtime, expr, env),
                Some(Binding::Reactive(_) | Binding::Value(Value::Signal(_)))
            ) {
                return true;
            }
            if let ExprKind::Call { callee, .. } = &expr.kind
                && let Some(Binding::Value(Value::Function(key))) = binding(runtime, callee, env)
                && visited.insert(key.clone())
                && let Some(group) = runtime.function_group(key)
            {
                for function in &group.clauses {
                    if let Some(scope) = runtime.globals.get(&function.module) {
                        let mut found = false;
                        for statement in &function.body {
                            statement.visit_expressions(&mut |expr| {
                                if !found {
                                    found = node(runtime, expr, scope, visited);
                                }
                            });
                        }
                        if found {
                            return true;
                        }
                    }
                }
            }
            false
        }
        fn inspect(
            runtime: &Runtime<'_, '_>,
            expr: &Expr,
            env: &Env,
            visited: &mut BTreeSet<String>,
        ) -> bool {
            let mut found = false;
            expr.visit_while(&mut |expr| {
                found |= node(runtime, expr, env, visited);
                !found
            });
            found
        }
        inspect(self, expr, env, &mut BTreeSet::new())
    }
    pub(super) fn retain_constructor_signals(
        &mut self,
        name: &str,
        value: &Value,
        args: &[(Option<String>, Expr)],
        observed: &[ObservedSignals],
        env: &Env,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let Value::Element(element) = value else {
            return Ok(());
        };
        let Some(builtin) = prelude::builtin(name) else {
            return Ok(());
        };
        for (index, ((label, expr), observed)) in args.iter().zip(observed).enumerate() {
            let label = label
                .as_deref()
                .or_else(|| {
                    builtin
                        .parameters
                        .get(index)
                        .map(|parameter| parameter.name)
                })
                .unwrap_or("");
            if label == "children" {
                if self.bind_list_children(element, expr, env)? {
                    continue;
                }
                if let ExprKind::List(children) = &expr.kind {
                    for child in children {
                        let native = matches!(&child.kind,ExprKind::Call {callee,..} if matches!(&callee.kind,ExprKind::Name(name) if prelude::builtin(name).is_some_and(|builtin|builtin.result==Type::Element)));
                        if !native && self.signal_in(child, env) {
                            return Err(self.error("S140","reactive child topology is unsupported; construct fixed native shapes with reactive scalar properties, or snapshot the child/factory result with let before placing it in children",child.span));
                        }
                    }
                    continue;
                }
                if matches!(&expr.kind,ExprKind::Name(name) if matches!(env.get(name),Some(Binding::Value(Value::List(_)))))
                {
                    continue;
                }
            }
            if !self.signal_in(expr, env) && observed.is_empty() {
                continue;
            }
            // Constructor shorthand supplies only axes without an explicit
            // override. Both axis bindings share the same expression node.
            let properties = if name == "Image" && label == "src" {
                vec![Property::Texture]
            } else if label == "overflow" {
                [
                    ("overflow_x", Property::OverflowX),
                    ("overflow_y", Property::OverflowY),
                ]
                .into_iter()
                .filter(|(axis, _)| {
                    !args.iter().enumerate().any(|(index, (name, _))| {
                        name.as_deref().or_else(|| {
                            builtin
                                .parameters
                                .get(index)
                                .map(|parameter| parameter.name)
                        }) == Some(*axis)
                    })
                })
                .map(|(_, property)| property)
                .collect::<Vec<_>>()
            } else {
                vec![property_name(label).ok_or_else(||self.error("S140",format!("'{label}' is not a retained animation property; snapshot dynamic topology/metadata with let, and retain scalar properties such as radius, width, position or fill"),expr.span))?]
            };
            if properties.is_empty() {
                continue;
            }
            self.signal_observations.push((false, observed.clone()));
            let result = if self.expression_mutates(expr) {
                Err(self.error("S140","retained properties require pure expressions; snapshot scheduling or mutating helper results with let first",expr.span))
            } else {
                self.lower_expression(expr, env, &BTreeMap::new(), 0)
            };
            self.signal_observations
                .pop()
                .expect("active retained argument observations");
            let node = result?;
            for property in properties {
                self.animation
                    .reactive
                    .bind(element.id.clone(), property, node)
                    .map_err(|error| self.error("S140", error, span))?;
            }
        }
        Ok(())
    }
    fn graph_node(&mut self, node: Node, span: Span) -> Result<NodeId, Diagnostic> {
        self.animation
            .reactive
            .set_native_byte_limit(self.limits.data.max_bytes)
            .map_err(|error| self.error("S140", error, span))?;
        let mut work = 0;
        let node = self
            .animation
            .reactive
            .insert_node_budgeted(
                node,
                self.limits.data.max_values,
                &mut work,
                self.limits
                    .max_native_work
                    .saturating_sub(self.logical_native_work),
            )
            .map_err(|e| self.error("S140", e, span))?;
        self.charge_native_work(work, span)?;
        Ok(node)
    }
    fn lower_binding(
        &mut self,
        binding: &Binding,
        span: Span,
        depth: usize,
    ) -> Result<NodeId, Diagnostic> {
        match binding {
            Binding::Value(Value::Signal(signal)) => {
                self.lower_binding(&Binding::Reactive(signal.id), span, depth)
            }
            Binding::Value(value) => self.graph_node(Node::Constant(value.clone()), span),
            Binding::Deferred(_) => {
                let value = self.binding_value(binding, span)?;
                self.graph_node(Node::Constant(value), span)
            }
            Binding::Module(_) => Err(self.error(
                "S140",
                "a module namespace cannot be retained as an animated property",
                span,
            )),
            Binding::Reactive(id) => {
                if let Some(node) = self.reactive.lowered.get(id) {
                    return Ok(*node);
                }
                let node = self.reactive.nodes.get(id).cloned().ok_or_else(|| {
                    self.error("S140", "signal refers to an expired local binding", span)
                })?;
                let node = match node {
                    Stored::Cell(_) => {
                        self.ensure_animated_cell(*id, span)?;
                        self.graph_node(Node::Cell(*id), span)?
                    }
                    Stored::Signal(signal) => {
                        if signal.dependencies.is_empty()
                            && !signal.temporal
                            && let Some(value) = signal.cached
                        {
                            let node = self.graph_node(Node::Constant(value), span)?;
                            self.reactive.lowered.insert(*id, node);
                            return Ok(node);
                        }
                        let previous = std::mem::replace(&mut self.module, signal.module.clone());
                        let result = self.lower_expression(
                            &signal.expression,
                            &signal.scope,
                            &BTreeMap::new(),
                            depth + 1,
                        );
                        self.module = previous;
                        result?
                    }
                };
                self.reactive.lowered.insert(*id, node);
                Ok(node)
            }
        }
    }
    fn lower_expression(
        &mut self,
        expr: &Expr,
        env: &Env,
        locals: &BTreeMap<String, NodeId>,
        depth: usize,
    ) -> Result<NodeId, Diagnostic> {
        if depth > self.limits.max_call_depth {
            return Err(self.error(
                "S303",
                "retained signal expression exceeds the bounded call-depth limit",
                expr.span,
            ));
        }
        if let Some(signal) = self.observed_signal(expr) {
            return self.lower_binding(&Binding::Reactive(signal.id), expr.span, depth + 1);
        }
        match &expr.kind {
            ExprKind::List(values) => {
                let values = values.iter().map(|value| self.lower_expression(value, env, locals, depth + 1)).collect::<Result<Vec<_>, _>>()?;
                self.graph_node(Node::List { values }, expr.span)
            }
            ExprKind::Record(fields) => {
                let fields = fields.iter().map(|(name, value)| self.lower_expression(value, env, locals, depth + 1).map(|node| (name.clone(), node))).collect::<Result<BTreeMap<_, _>, _>>()?;
                self.graph_node(Node::Record { fields }, expr.span)
            }
            ExprKind::Template(parts) => {
                let parts = parts.iter().map(|part| {
                    Ok(match part {
                        sanim_core::TemplatePart::Text(text) => sanim_core::TemplatePart::Text(text.clone()),
                        sanim_core::TemplatePart::Hole { value, format, conversion } => sanim_core::TemplatePart::Hole {
                            value: self.lower_expression(value, env, locals, depth + 1)?,
                            format: format.as_ref().map(|format| self.lower_expression(format, env, locals, depth + 1)).transpose()?,
                            conversion: *conversion,
                        },
                    })
                }).collect::<Result<Vec<_>, Diagnostic>>()?;
                self.graph_node(Node::Template { parts: parts.into() }, expr.span)
            }
            ExprKind::Name(name)=>{
                if let Some(node)=locals.get(name){return Ok(*node);}
                if let Some(value)=prelude::values().get(name.as_str()){return self.graph_node(Node::Constant(value.clone()),expr.span);}
                let binding=env.get(name).cloned().ok_or_else(||self.error("S120",format!("unknown retained expression name '{name}'"),expr.span))?;
                self.lower_binding(&binding,expr.span,depth)
            }
            ExprKind::Unary(op,value)=>{let value=self.lower_expression(value,env,locals,depth+1)?;self.graph_node(Node::Unary{op:op.clone(),value},expr.span)},
            ExprKind::Binary(op,left,right)=>{let left=self.lower_expression(left,env,locals,depth+1)?;let right=self.lower_expression(right,env,locals,depth+1)?;self.graph_node(Node::Binary{op:op.clone(),left,right},expr.span)},
            ExprKind::Index(base,index)=>{
                let index=crate::list_functions::constant_count(Some(index),None).ok_or_else(||self.error("S140","retained List indexing requires a constant nonnegative index",index.span))?;
                let list=self.lower_expression(base,env,locals,depth+1)?;
                self.graph_node(Node::Index{list,index},expr.span)
            }
            ExprKind::Member(base,name)=>{
                if let ExprKind::Name(alias)=&base.kind && let Some(Binding::Module(module))=env.get(alias){let binding=self.globals.get(module).and_then(|scope|scope.get(name)).cloned().ok_or_else(||self.error("S120",format!("module '{alias}' has no export '{name}'"),expr.span))?;return self.lower_binding(&binding,expr.span,depth);}
                match self.expression(base,env)? {
                    Value::Record(fields) if !self.signal_in(base,env)=>{
                        let value=fields.get(name).cloned().ok_or_else(||self.error("S140",format!("unknown record field '{name}'"),expr.span))?;self.graph_node(Node::Constant(value),expr.span)
                    }
                    Value::Record(_)=>{
                        let record=self.lower_expression(base,env,locals,depth+1)?;
                        self.graph_node(Node::Field{record,name:name.clone()},expr.span)
                    }
                    Value::Element(element)=>{
                        let property=property_name(name).ok_or_else(||self.error("S140","unsupported retained element property",expr.span))?;
                        self.animation.reactive.binding(&element.id,property).ok_or_else(||self.error("S140","retaining an element's ordinary motion property is unsupported; derive both properties from the same mut input, or snapshot this getter with let",expr.span))
                    }
                    _ if !self.signal_in(base,env)=>{let value=self.expression(expr,env)?;self.graph_node(Node::Constant(value),expr.span)},
                    _=>Err(self.error("S140","retained structured field access requires scalar/List inputs; snapshot the record with let or expose its components as scalar cells",expr.span)),
                }
            }
            ExprKind::Call{callee,args,..}=>{
                let native_call = self.loaded.iter().find(|module|module.id==self.module)
                    .and_then(|module|crate::timing::builtin_call(callee,args,&module.checked.native_methods))
                    .and_then(|call|sanim_core::code::values::Native::from_name(call.builtin.name).map(|operation| {
                        let expressions=call.builtin.parameters.iter().enumerate()
                            .filter_map(|(index,parameter)|call.argument(index).map(|value|(parameter.name.to_owned(),value.clone())))
                            .collect::<Vec<_>>();
                        (operation,expressions)
                    }));
                if let Some((operation,expressions))=native_call {
                    let arguments=expressions.into_iter().map(|(name,value)|self.lower_expression(&value,env,locals,depth+1).map(|node|(name,node))).collect::<Result<BTreeMap<_,_>,_>>()?;
                    return self.graph_node(Node::NativeConfig{operation,arguments},expr.span);
                }
                let args = args.iter();
                if let ExprKind::Name(name)=&callee.kind && prelude::builtin(name).is_some(){
                    if math::functions::function(name).is_none() && math::linear::list_function(name).is_none(){
                        let mut names=BTreeSet::new();dependencies(expr,&mut names);
                        if !self.has_live_observation(expr) && names.iter().all(|name|!locals.contains_key(name)&&!env.get(name).is_some_and(Binding::is_dynamic)) {let value=self.expression(expr,env)?;return self.graph_node(Node::Constant(value),expr.span);}
                    }
                    let builtin=prelude::builtin(name).expect("known native");
                    let mut supplied=BTreeMap::new();let mut positional=0;
                    for (label,value) in args {let label=if let Some(label)=label{label.clone()}else{let label=builtin.parameters.get(positional).ok_or_else(||self.error("S140","too many retained math arguments",value.span))?.name.to_owned();positional+=1;label};supplied.insert(label,self.lower_expression(value,env,locals,depth+1)?);}
                    let args=builtin.parameters.iter().filter_map(|parameter|supplied.remove(parameter.name)).collect();
                    return self.graph_node(Node::Math{name:name.clone(),args},expr.span);
                }
                let Value::Function(key)=self.expression(callee,env)? else{return Err(self.error("S140","retained signals require direct pure helper calls",expr.span));};
                let group=self.function_group(&key).cloned().ok_or_else(||self.error("S140","retained constructor/data calls must be snapshotted with let first",expr.span))?;
                if group.clauses.len()!=1 {return Err(self.error("S140","retained animated helpers require one straight-line clause; snapshot a guarded helper call with let before retaining it",expr.span));}
                // Lowering observes code even when the initial value sample short-circuits it.
                self.record_function(&key,&group);
                let function=group.clauses[0].clone();
                let mut inputs=BTreeMap::new();
                for (index,(label,value)) in args.enumerate(){let name=label.as_ref().or_else(||function.parameters.get(index).map(|parameter| &parameter.name)).ok_or_else(||self.error("S140","invalid retained helper argument",value.span))?;inputs.insert(name.clone(),self.lower_expression(value,env,locals,depth+1)?);}
                let scope=group.captures.clone().unwrap_or_else(||self.globals.get(&function.module).cloned().unwrap_or_default());
                let previous=std::mem::replace(&mut self.module,function.module.clone());
                let result=(||{
                    for parameter in &function.parameters {let name=&parameter.name; let default=&parameter.default;if !inputs.contains_key(name){let default=default.as_ref().ok_or_else(||self.error("S140",format!("retained helper is missing argument '{name}'"),expr.span))?;inputs.insert(name.clone(),self.lower_expression(default,&scope,&inputs,depth+1)?);}}
                    for statement in &function.body {
                        match &statement.kind {
                            StatementKind::Bind{kind:BindingKind::Let|BindingKind::Mut,name,value,..}=>{let node=self.lower_expression(value,&scope,&inputs,depth+1)?;inputs.insert(name.clone(),node);},
                            StatementKind::Assign{name,value} if inputs.contains_key(name)=>{let node=self.lower_expression(value,&scope,&inputs,depth+1)?;inputs.insert(name.clone(),node);},
                            StatementKind::Return(Some(value))=>return self.lower_expression(value,&scope,&inputs,depth+1),
                            _=>return Err(self.error("S140","retained animated helpers require straight-line pure arithmetic and return; snapshot branching/data work with let or express it as signal arithmetic",statement.span)),
                        }
                    }
                    Err(self.error("S140","retained helper must return a supported scalar, numeric List, or Color value",expr.span))
                })();self.module=previous;result
            }
            ExprKind::Number(_)|ExprKind::String(_)|ExprKind::Bool(_)=>{let value=self.expression(expr,env)?;self.graph_node(Node::Constant(value),expr.span)},
            _=>Err(self.error("S140","this signal operation cannot be retained for native playback; use scalar math, numeric Lists, colors, and straight-line pure helpers, or snapshot it with let",expr.span)),
        }
    }
}

pub(super) fn property_name(name: &str) -> Option<Property> {
    if let Some((_, property)) = sanim_core::code::view::INPUTS
        .iter()
        .chain(sanim_core::voxel::INPUTS)
        .find(|(input, _)| *input == name)
    {
        return Some(*property);
    }
    Some(match name {
        "position" => Property::Position,
        "position_mode" => Property::PositionMode,
        "scroll" => Property::Scroll,
        "white_space" => Property::WhiteSpace,
        "overflow_wrap" => Property::OverflowWrap,
        "overflow" => Property::Overflow,
        "overflow_x" => Property::OverflowX,
        "overflow_y" => Property::OverflowY,
        "rotation" => Property::Rotation,
        "scale" => Property::Scale,
        "opacity" => Property::Opacity,
        "fill" => Property::Fill,
        "radius" => Property::Radius,
        "width" => Property::Width,
        "height" => Property::Height,
        "text" => Property::Text,
        "font" => Property::Font,
        "font_size" => Property::FontSize,
        "x_align" => Property::XAlign,
        "y_align" => Property::YAlign,
        _ => return None,
    })
}
