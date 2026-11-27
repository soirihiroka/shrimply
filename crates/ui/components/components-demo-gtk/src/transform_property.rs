use std::{cell::Cell, rc::Rc};

use shrimply_components_core::layered::{
    LayeredEdit, LayeredPropertyController, component_changes,
};
use shrimply_components_gtk::ui::{
    ExpressionEditor, FrameGraph, InspectorGraphProperty, NumberPickerHandle,
};

pub fn graph(label: &str, value: f64, log: Rc<dyn Fn(String)>) -> FrameGraph {
    let label = label.to_string();
    FrameGraph::with_actions(
        shrimply_components_demo_core::property_graph_state(value),
        move |_| log(format!("{label} keyframe action")),
    )
}

pub fn pair_graph(label: &str, values: [f64; 2], log: Rc<dyn Fn(String)>) -> FrameGraph {
    let label = label.to_string();
    FrameGraph::with_components(
        shrimply_components_demo_core::property_graph_components(&values, 0),
        move |_| log(format!("{label} keyframe action")),
    )
}

pub fn edit_handler(
    controller: LayeredPropertyController,
    graph: FrameGraph,
) -> impl Fn(f64) + 'static {
    move |value| {
        if let LayeredEdit::Keyframe(value) = controller.edit(value) {
            graph.edit_value(value);
        }
    }
}

pub fn pair_edit_handler(
    controller: LayeredPropertyController,
    graph: FrameGraph,
    value: Rc<Cell<[f64; 2]>>,
) -> impl Fn([f64; 2], usize) + 'static {
    move |next, component| {
        let previous = value.replace(next);
        match controller.edit_component(next, component) {
            LayeredEdit::Base(_) => graph.activate_component(component),
            LayeredEdit::Keyframe(_) => {
                graph.edit_component_values(component, &component_changes(previous, next));
            }
        }
    }
}

pub struct PropertyConfig<'a> {
    pub label: &'a str,
    pub initial_value: f64,
    pub modes: (bool, bool),
}

pub struct PairPropertyConfig<'a> {
    pub label: &'a str,
    pub initial_values: [f64; 2],
    pub modes: (bool, bool),
}

pub fn scalar_property(
    config: PropertyConfig<'_>,
    editor: &gtk::Widget,
    handle: &NumberPickerHandle,
    graph: FrameGraph,
    controller: LayeredPropertyController,
    log: Rc<dyn Fn(String)>,
) -> InspectorGraphProperty {
    let handle = handle.clone();
    property(config, editor, graph, controller, log, move |value| {
        handle.set_f64(value);
    })
}

pub fn pair_property(
    config: PairPropertyConfig<'_>,
    editor: &gtk::Widget,
    handles: [&NumberPickerHandle; 2],
    graph: FrameGraph,
    controller: LayeredPropertyController,
    value: Rc<Cell<[f64; 2]>>,
    log: Rc<dyn Fn(String)>,
) -> InspectorGraphProperty {
    value.set(config.initial_values);
    let handles = [handles[0].clone(), handles[1].clone()];
    for (handle, initial) in handles.iter().zip(config.initial_values) {
        handle.set_f64(initial);
    }
    let graph_controller = controller.clone();
    property(
        PropertyConfig {
            label: config.label,
            initial_value: config.initial_values[0],
            modes: config.modes,
        },
        editor,
        graph,
        controller,
        log,
        move |graph_value| {
            let component = graph_controller.active_component().min(1);
            let mut next = value.get();
            next[component] = graph_value;
            value.set(next);
            handles[component].set_f64(graph_value);
        },
    )
}

pub fn reset_pair(
    graph: &FrameGraph,
    controller: &LayeredPropertyController,
    handles: [&NumberPickerHandle; 2],
    value: &Cell<[f64; 2]>,
    initial_values: [f64; 2],
) {
    controller.select_component::<2>(0);
    value.set(initial_values);
    graph.replace_components(shrimply_components_demo_core::property_graph_components(
        &initial_values,
        0,
    ));
    for (handle, initial) in handles.into_iter().zip(initial_values) {
        handle.set_f64(initial);
    }
}

fn property(
    config: PropertyConfig<'_>,
    editor: &gtk::Widget,
    graph: FrameGraph,
    controller: LayeredPropertyController,
    log: Rc<dyn Fn(String)>,
    on_graph_value: impl Fn(f64) + 'static,
) -> InspectorGraphProperty {
    on_graph_value(config.initial_value);
    let status_controller = controller.clone();
    graph.connect_status({
        move |status| {
            if status_controller.keyframes() {
                on_graph_value(status.value);
            }
        }
    });
    let expression_label = config.label.to_string();
    let expression = ExpressionEditor::new(
        shrimply_components_demo_core::EXPRESSION_SOURCE,
        Some("sanim"),
        &shrimply_components_demo_core::expression_output(
            shrimply_components_demo_core::EXPRESSION_SOURCE,
        ),
        {
            move |value| {
                log(format!(
                    "{expression_label} expression edited ({} chars)",
                    value.len()
                ));
                shrimply_components_demo_core::expression_output(&value)
            }
        },
    );
    let property = InspectorGraphProperty::new(config.label, editor, graph, expression, controller);
    property.set_keyframes_active(config.modes.0);
    property.set_expression_active(config.modes.1);
    property
}
