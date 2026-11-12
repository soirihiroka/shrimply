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
                .set_cell(id, cell.value.clone())
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
    pub(super) fn animate_binding(
        &mut self,
        name: &str,
        value: &Expr,
        duration: &Expr,
        easing: Option<&Expr>,
        env: &Env,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let Some(Binding::Reactive(id)) = env.get(name) else {
            return Err(self.error(
                "S140",
                format!("animate target '{name}' must be declared with mut"),
                span,
            ));
        };
        let id = *id;
        self.refresh_cell(id, span)?;
        let Some(Stored::Cell(cell)) = self.reactive.nodes.get(&id).cloned() else {
            return Err(self.error(
                "S140",
                format!("'{name}' is a read-only signal; animate its mut input instead"),
                span,
            ));
        };
        let target = self
            .expression(value, env)?
            .with_type(&cell.ty)
            .map_err(|e| self.error("S140", e, value.span))?;
        let duration = self
            .expression(duration, env)?
            .number()
            .map_err(|e| self.error("S304", e, duration.span))?;
        if duration.unit != Unit::Duration || duration.value <= Fraction::from(0) {
            return Err(self.error(
                "S304",
                "animate duration must be a positive bounded Duration such as 1s",
                span,
            ));
        }
        let end = math::timeline_end(self.cursor, duration.value)
            .map_err(|e| self.error("S304", e, span))?;
        let easing = if let Some(easing) = easing {
            let value = self.expression(easing, env)?;
            Easing::parse(
                value
                    .string()
                    .map_err(|e| self.error("S140", e, easing.span))?,
            )
            .map_err(|e| self.error("S140", e, easing.span))?
        } else {
            Easing::Linear
        };
        self.ensure_animated_cell(id, span)?;
        let start = self.cursor;
        self.animation
            .reactive
            .animate_cell(id, start, end, target, easing)
            .map_err(|e| self.error("S140", e, span))?;
        self.cursor = end;
        self.animation.duration = self.animation.duration.max(end);
        self.refresh_cell(id, span)?;
        self.map_time(start, span, TimeMapKind::Statement)?;
        Ok(())
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
            if matches!(binding(runtime, expr, env), Some(Binding::Reactive(_))) {
                return true;
            }
            if let ExprKind::Call { callee, .. } = &expr.kind
                && let Some(Binding::Value(Value::Function(key))) = binding(runtime, callee, env)
                && visited.insert(key.clone())
                && let Some(function) = runtime.functions.get(key)
                && let Some(scope) = runtime.globals.get(&function.module)
            {
                let mut found = false;
                for statement in &function.body {
                    statement.visit_expressions(&mut |expr| {
                        if !found {
                            found = node(runtime, expr, scope, visited);
                        }
                    });
                }
                return found;
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
    pub(super) fn check_shape_helper_inputs(
        &self,
        key: &str,
        args: &[(Option<String>, Expr)],
        env: &Env,
        span: Span,
    ) -> Result<(), Diagnostic> {
        fn shape(ty: &Type) -> bool {
            match ty {
                Type::Element => true,
                Type::List(inner) | Type::Option(inner) | Type::Array(inner) => shape(inner),
                Type::Record(fields) => fields.values().any(shape),
                Type::Union(types) => types.iter().any(shape),
                _ => false,
            }
        }
        let Some(function) = self.functions.get(key) else {
            return Ok(());
        };
        let result = self
            .loaded
            .iter()
            .find(|module| module.id == function.module)
            .and_then(|module| module.checked.exports.get(&function.name));
        if matches!(result,Some(Type::Function(signature)) if shape(&signature.result))
            && args.iter().any(|(_, expr)| self.signal_in(expr, env))
        {
            return Err(self.error("S140","shape-helper parameters are value snapshots and cannot retain reactive arguments; construct the shape directly, capture the mut/sig in the helper, or snapshot the input with let before calling the helper",span));
        }
        Ok(())
    }
    pub(super) fn retain_constructor_signals(
        &mut self,
        name: &str,
        value: &Value,
        args: &[(Option<String>, Expr)],
        env: &Env,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let Value::Element(element) = value else {
            return Ok(());
        };
        let Some(builtin) = prelude::builtin(name) else {
            return Ok(());
        };
        for (index, (label, expr)) in args.iter().enumerate() {
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
            if !self.signal_in(expr, env) {
                continue;
            }
            let property=property_name(label).ok_or_else(||self.error("S140",format!("'{label}' is not a retained animation property; snapshot dynamic topology/metadata with let, and retain scalar properties such as radius, width, position or fill"),expr.span))?;
            if self.expression_mutates(expr) || self.expression_schedules(expr) {
                return Err(self.error("S140","retained properties require pure expressions; snapshot scheduling or mutating helper results with let first",expr.span));
            }
            let node = self.lower_expression(expr, env, &BTreeMap::new(), 0)?;
            self.animation
                .reactive
                .bind(element.id.clone(), property, node)
                .map_err(|e| self.error("S140", e, span))?;
        }
        Ok(())
    }
    fn graph_node(&mut self, node: Node, span: Span) -> Result<NodeId, Diagnostic> {
        self.animation
            .reactive
            .insert_node(node, self.limits.data.max_values)
            .map_err(|e| self.error("S140", e, span))
    }
    fn lower_binding(
        &mut self,
        binding: &Binding,
        span: Span,
        depth: usize,
    ) -> Result<NodeId, Diagnostic> {
        match binding {
            Binding::Value(value) => self.graph_node(Node::Constant(value.clone()), span),
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
        match &expr.kind {
            ExprKind::Name(name)=>{
                if let Some(node)=locals.get(name){return Ok(*node);}
                if let Some(value)=prelude::values().get(name.as_str()){return self.graph_node(Node::Constant(value.clone()),expr.span);}
                let binding=env.get(name).cloned().ok_or_else(||self.error("S120",format!("unknown retained expression name '{name}'"),expr.span))?;
                self.lower_binding(&binding,expr.span,depth)
            }
            ExprKind::Unary(op,value)=>{let value=self.lower_expression(value,env,locals,depth+1)?;self.graph_node(Node::Unary{op:op.clone(),value},expr.span)},
            ExprKind::Binary(op,left,right)=>{let left=self.lower_expression(left,env,locals,depth+1)?;let right=self.lower_expression(right,env,locals,depth+1)?;self.graph_node(Node::Binary{op:op.clone(),left,right},expr.span)},
            ExprKind::Member(base,name)=>{
                if let ExprKind::Name(alias)=&base.kind && let Some(Binding::Module(module))=env.get(alias){let binding=self.globals.get(module).and_then(|scope|scope.get(name)).cloned().ok_or_else(||self.error("S120",format!("module '{alias}' has no export '{name}'"),expr.span))?;return self.lower_binding(&binding,expr.span,depth);}
                if let ExprKind::Name(local)=&base.kind && let Some(vector)=locals.get(local) {
                    let index=["x","y","z","w"].iter().position(|component|*component==name.as_str());
                    if let Some(index)=index {return self.graph_node(Node::Component{vector:*vector,index},expr.span);}
                    return Err(self.error("S140","retained helper field access supports vector x/y/z/w components; snapshot other structured fields before calling the helper",expr.span));
                }
                match self.expression(base,env)? {
                    Value::Vec2(_)|Value::Vec3(_)|Value::Vec4(_)=>{
                        let vector=self.lower_expression(base,env,locals,depth+1)?;
                        let index=["x","y","z","w"].iter().position(|component|*component==name).ok_or_else(||self.error("S140","unknown vector component",expr.span))?;
                        self.graph_node(Node::Component{vector,index},expr.span)
                    }
                    Value::Record(fields) if !self.signal_in(base,env)=>{
                        let value=fields.get(name).cloned().ok_or_else(||self.error("S140",format!("unknown record field '{name}'"),expr.span))?;self.graph_node(Node::Constant(value),expr.span)
                    }
                    Value::Element(element)=>{
                        let property=property_name(name).ok_or_else(||self.error("S140","unsupported retained element property",expr.span))?;
                        self.animation.reactive.binding(&element.id,property).ok_or_else(||self.error("S140","retaining an element's ordinary motion property is unsupported; derive both properties from the same mut input, or snapshot this getter with let",expr.span))
                    }
                    _ if !self.signal_in(base,env)=>{let value=self.expression(expr,env)?;self.graph_node(Node::Constant(value),expr.span)},
                    _=>Err(self.error("S140","retained structured field access requires scalar/vector inputs; snapshot the record with let or expose its components as scalar cells",expr.span)),
                }
            }
            ExprKind::Call{callee,args,..}=>{
                if let ExprKind::Name(name)=&callee.kind && prelude::builtin(name).is_some(){
                    if matches!(name.as_str(),"Vec2"|"Vec3"|"Vec4") {
                        let count=match name.as_str(){"Vec2"=>2,"Vec3"=>3,_=>4};
                        let components=["x","y","z","w"];let mut values=vec![None;count];let mut positional=0;
                        for (label,value) in args {let index=if let Some(label)=label {components.iter().position(|name|name==label).ok_or_else(||self.error("S140","unknown vector component",value.span))?}else{let index=positional;positional+=1;index};if index>=count{return Err(self.error("S140","too many vector components",value.span));}values[index]=Some(self.lower_expression(value,env,locals,depth+1)?);}
                        let unit=values.iter().flatten().next().and_then(|node|self.animation.reactive.node_type(*node)).and_then(math::operations::scalar_unit).unwrap_or(Unit::Number);
                        let mut complete=Vec::with_capacity(count);for value in values {complete.push(if let Some(value)=value {value}else{self.graph_node(Node::Constant(Value::Number(Number{value:0.into(),unit})),expr.span)?});}
                        return self.graph_node(Node::Vector{values:complete},expr.span);
                    }
                    if math::functions::function(name).is_none(){
                        let mut names=BTreeSet::new();dependencies(expr,&mut names);
                        if names.iter().all(|name|!locals.contains_key(name)&&!matches!(env.get(name),Some(Binding::Reactive(_)|Binding::Module(_)))) {let value=self.expression(expr,env)?;return self.graph_node(Node::Constant(value),expr.span);}
                    }
                    let builtin=prelude::builtin(name).expect("known native");
                    let mut supplied=BTreeMap::new();let mut positional=0;
                    for (label,value) in args {let label=if let Some(label)=label{label.clone()}else{let label=builtin.parameters.get(positional).ok_or_else(||self.error("S140","too many retained math arguments",value.span))?.name.to_owned();positional+=1;label};supplied.insert(label,self.lower_expression(value,env,locals,depth+1)?);}
                    let args=builtin.parameters.iter().filter_map(|parameter|supplied.remove(parameter.name)).collect();
                    return self.graph_node(Node::Math{name:name.clone(),args},expr.span);
                }
                let Value::Function(key)=self.expression(callee,env)? else{return Err(self.error("S140","retained signals require direct pure helper calls",expr.span));};
                let function=self.functions.get(&key).cloned().ok_or_else(||self.error("S140","retained constructor/data calls must be snapshotted with let first",expr.span))?;
                let mut inputs=BTreeMap::new();
                for (index,(label,value)) in args.iter().enumerate(){let name=label.as_ref().or_else(||function.parameters.get(index).map(|(name,..)|name)).ok_or_else(||self.error("S140","invalid retained helper argument",value.span))?;inputs.insert(name.clone(),self.lower_expression(value,env,locals,depth+1)?);}
                let scope=self.globals.get(&function.module).cloned().unwrap_or_default();
                let previous=std::mem::replace(&mut self.module,function.module.clone());
                let result=(||{
                    for (name,_,default) in &function.parameters {if !inputs.contains_key(name){let default=default.as_ref().ok_or_else(||self.error("S140",format!("retained helper is missing argument '{name}'"),expr.span))?;inputs.insert(name.clone(),self.lower_expression(default,&scope,&inputs,depth+1)?);}}
                    for statement in &function.body {
                        match &statement.kind {
                            StatementKind::Bind{kind:BindingKind::Let|BindingKind::Mut,name,value,..}=>{let node=self.lower_expression(value,&scope,&inputs,depth+1)?;inputs.insert(name.clone(),node);},
                            StatementKind::Assign{name,value} if inputs.contains_key(name)=>{let node=self.lower_expression(value,&scope,&inputs,depth+1)?;inputs.insert(name.clone(),node);},
                            StatementKind::Return(value)=>return self.lower_expression(value,&scope,&inputs,depth+1),
                            _=>return Err(self.error("S140","retained animated helpers require straight-line pure arithmetic and return; snapshot branching/data work with let or express it as signal arithmetic",statement.span)),
                        }
                    }
                    Err(self.error("S140","retained helper must return a supported scalar/vector/color value",expr.span))
                })();self.module=previous;result
            }
            ExprKind::Number(_)|ExprKind::String(_)|ExprKind::Bool(_)|ExprKind::Color(_)=>{let value=self.expression(expr,env)?;self.graph_node(Node::Constant(value),expr.span)},
            _=>Err(self.error("S140","this signal operation cannot be retained for native playback; use scalar math, vectors, colors, and straight-line pure helpers, or snapshot it with let",expr.span)),
        }
    }
}

pub(super) fn property_name(name: &str) -> Option<Property> {
    Some(match name {
        "position" => Property::Position,
        "rotation" => Property::Rotation,
        "scale" => Property::Scale,
        "opacity" => Property::Opacity,
        "fill" => Property::Fill,
        "radius" => Property::Radius,
        "width" => Property::Width,
        "height" => Property::Height,
        _ => return None,
    })
}
