use gtk::prelude::*;
use shrimply_components_gtk::tr;
use uuid::Uuid;

use super::timeline_value::color::{ColorAccess, ColorTarget, color_control};
use super::timeline_value::scalar::{ScalarAccess, ScalarSpec, ScalarTarget, scalar_control};
use super::timeline_value::vector::vec2::{VecAccess, VecSpec, VecTarget, vec_control};
use crate::InspectedItem as SelectedItem;
use crate::player_state::{self, ProjectChange};
use shrimply_project_document::project::{Project, Time};
use shrimply_project_document::project::{VisualAlphaMaskTarget, VisualModifier};
use shrimply_property_model::timeline_value::{TimelineStep, TimelineValue};
use shrimply_visual_modifiers::{
    ModifierEffect, ModifierModel, RasterModifierEffect, scene_3d::Scene3dModifierEffect,
};

use super::{
    InspectorContext,
    item::{InspectorListItem, flat},
};

struct ModifierStepTarget<T: TimelineStep> {
    modifier_id: Uuid,
    commit_name: &'static str,
    get: fn(&ModifierEffect) -> Option<&TimelineValue<T>>,
    get_mut: fn(&mut ModifierEffect) -> Option<&mut TimelineValue<T>>,
}

fn step_row<T: TimelineStep>(
    label: &str,
    value: &TimelineValue<T>,
    context: &InspectorContext,
    target: ModifierStepTarget<T>,
) -> gtk::Widget {
    let timeline_id = value.id;
    let ModifierStepTarget {
        modifier_id,
        commit_name,
        get,
        get_mut,
    } = target;
    crate::timeline_value::step::step_control(
        label,
        value,
        context,
        crate::timeline_value::step::StepTarget::new(
            move |project, key| {
                project
                    .video_item(&key)?
                    .modifiers
                    .iter()
                    .find(|modifier| modifier.id == modifier_id)
                    .and_then(|modifier| get(&modifier.effect))
                    .filter(|timeline| timeline.id == timeline_id)
            },
            move |project, key| {
                project
                    .video_item_mut(&key)?
                    .modifiers
                    .iter_mut()
                    .find(|modifier| modifier.id == modifier_id)
                    .and_then(|modifier| get_mut(&mut modifier.effect))
                    .filter(|timeline| timeline.id == timeline_id)
            },
            commit_name,
            ProjectChange {
                video: true,
                inspector: true,
                ..Default::default()
            },
        )
        .refresh_inspector_on_value_change(),
    )
}

#[path = "modifiers/add_menu.rs"]
mod add_menu;

#[path = "modifiers/cache.rs"]
mod cache;
#[path = "modifiers/dithering.rs"]
mod dithering;
#[path = "modifiers/mask.rs"]
mod mask;
#[path = "modifiers/object_3d.rs"]
mod object_3d;
#[path = "modifiers/sam2.rs"]
mod sam2;
#[path = "modifiers/text_3d.rs"]
mod text_3d;
#[path = "modifiers/transparent_fill.rs"]
mod transparent_fill;
#[path = "modifiers/vectorize.rs"]
mod vectorize;

pub(super) fn items(context: &InspectorContext) -> Vec<InspectorListItem> {
    let document = shrimply_inspector_document::basic(context.inspector_core.snapshot());
    let mut items = document
        .categories
        .into_iter()
        .find(|category| category.key == "visual")
        .expect("visual inspector category")
        .items
        .into_iter()
        .filter_map(|item| match item {
            shrimply_inspector_document::InspectorListItem::Item(item)
                if item.presentation.key.starts_with("modifier:") =>
            {
                Some(InspectorListItem::Item(Box::new(
                    super::item::DocumentItem::new(*item, context, controls),
                )))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    items.push(flat(modifier_buttons(context)));
    items
}

fn controls(
    section: &shrimply_inspector_core::InspectorSection,
    context: &InspectorContext,
) -> Vec<gtk::Widget> {
    let id = section
        .controls
        .iter()
        .find_map(|control| control.target_id)
        .expect("modifier controls have an owner");
    let modifier = {
        let project = context.project.borrow();
        let item = project
            .video_item(
                context
                    .selected_item
                    .as_ref()
                    .expect("selected visual item"),
            )
            .expect("visual item still exists");
        item.modifiers
            .iter()
            .find(|modifier| modifier.id == id)
            .expect("modifier still exists")
            .clone()
    };
    let out = gtk::Box::new(gtk::Orientation::Vertical, 8);
    match &modifier.effect {
        ModifierEffect::Scene3d(effect) => match &**effect {
            Scene3dModifierEffect::Object(value) => object_3d::add_rows(value, &out, id, context),
            Scene3dModifierEffect::Text(value) => text_3d::add_rows(value, &out, id, context),
            _ => return crate::control::section(section, context),
        },
        ModifierEffect::Raster(effect) => match &**effect {
            RasterModifierEffect::Cache(value) => cache::add_rows(value, &out, id, context),
            RasterModifierEffect::Dithering(value) => dithering::add_rows(value, &out, id, context),
            RasterModifierEffect::Mask(value) => mask::add_rows(value, &out, id, context),
            RasterModifierEffect::Sam2(value) => sam2::add_rows(value, &out, id, context),
            RasterModifierEffect::TransparentFill(value) => {
                transparent_fill::add_rows(value, &out, id, context)
            }
            _ => return crate::control::section(section, context),
        },
        ModifierEffect::Vectorize(value) => vectorize::add_rows(value, &out, id, context),
        _ => return crate::control::section(section, context),
    }
    if matches!(&modifier.effect, ModifierEffect::Raster(effect) if !matches!(&**effect, RasterModifierEffect::Cache(_)))
    {
        out.append(&crate::alpha_mask::widget(
            VisualAlphaMaskTarget::Modifier(id),
            context,
        ));
    }
    vec![out.upcast()]
}

fn modifier_buttons(context: &InspectorContext) -> gtk::Widget {
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    buttons.set_halign(gtk::Align::Center);
    buttons.append(&add_menu::button(context));
    let targets = context
        .selected_item
        .clone()
        .into_iter()
        .collect::<Vec<_>>();
    let sensitive = {
        let project = context.project.borrow();
        context
            .property_clipboard
            .borrow()
            .can_append_modifiers(&project, &targets)
    };
    if sensitive {
        let paste = gtk::Button::builder()
            .icon_name("edit-paste-symbolic")
            .tooltip_text(tr!("Paste Modifier").as_ref())
            .build();
        let context = context.detached();
        paste.connect_clicked(move |_| paste_modifiers(&context));
        buttons.append(&paste);
    }
    buttons.upcast()
}

fn paste_modifiers(context: &InspectorContext) {
    let Some(target) = context.selected_item.clone() else {
        return;
    };
    let result = {
        let mut project = context.project.borrow_mut();
        let result = context
            .property_clipboard
            .borrow()
            .append_modifiers(&mut project, &[target]);
        if result.changed {
            shrimply_project_document::project::commit_edit(&project, "paste-item-modifiers");
        }
        result
    };
    if !result.changed {
        return;
    }
    let message = if result.modifiers_added == 1 {
        tr!("1 effect pasted").into_owned()
    } else {
        shrimply_components_gtk::i18n::text_args(
            "%{count} effects pasted",
            &[("count", result.modifiers_added.to_string())],
        )
    };
    shrimply_components_gtk::toast::show_confirmation_text_for_widget(
        &context.category_bar,
        &message,
    );
    player_state::refresh_project(
        &context.player_state,
        ProjectChange {
            video: result.video,
            inspector: true,
            ..Default::default()
        },
    );
}

pub(super) fn add_effect(effect: ModifierEffect, context: &InspectorContext) {
    let Some(key) = context.selected_item.clone() else {
        return;
    };
    let mut project = context.project.borrow_mut();
    let Some(item) = project.video_item_mut(&key) else {
        return;
    };
    let modifier = VisualModifier::new(effect);
    let id = modifier.id;
    let mut modifiers = item.modifiers.clone();
    modifiers.push(modifier);
    if !shrimply_inspector_core::visual_modifiers::modifier_chain_is_valid(item, &modifiers) {
        return;
    }
    item.modifiers = modifiers;
    shrimply_project_document::project::commit_edit(&project, "add-visual-modifier");
    drop(project);
    context.list_state.borrow_mut().set_expanded(
        &context.list_target,
        &format!("modifier:{id}"),
        true,
    );
    player_state::refresh_project(
        &context.player_state,
        ProjectChange {
            video: true,
            inspector: true,
            ..Default::default()
        },
    );
}

#[derive(Clone, Copy)]
pub(crate) struct ScalarOptions {
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub unit: Option<&'static str>,
    pub rotating: bool,
}

pub(crate) fn scalar_row(
    label: &str,
    value: &TimelineValue<f32>,
    modifier_id: Uuid,
    options: ScalarOptions,
    context: &InspectorContext,
) -> gtk::Widget {
    scalar_row_for::<ContinuousScalar>(label, value, modifier_id, options, context)
}

pub(crate) fn integer_scalar_row(
    label: &str,
    value: &TimelineValue<f32>,
    modifier_id: Uuid,
    options: ScalarOptions,
    context: &InspectorContext,
) -> gtk::Widget {
    scalar_row_for::<IntegerScalar>(label, value, modifier_id, options, context)
}

trait ScalarMode {
    const DRAG_STEP: f64;
    const DIGITS: usize;
    const INTEGER: bool;
    fn clamp(options: ScalarOptions) -> fn(f32) -> f32;
}

struct ContinuousScalar;

impl ScalarMode for ContinuousScalar {
    const DRAG_STEP: f64 = 0.01;
    const DIGITS: usize = 2;
    const INTEGER: bool = false;

    fn clamp(options: ScalarOptions) -> fn(f32) -> f32 {
        match (options.minimum, options.maximum) {
            (Some(0.0), Some(1.0)) => |value| value.clamp(0.0, 1.0),
            (Some(0.0), Some(2.0)) => |value| value.clamp(0.0, 2.0),
            (Some(0.0), Some(100.0)) => |value| value.clamp(0.0, 100.0),
            (Some(-1.0), Some(1.0)) => |value| value.clamp(-1.0, 1.0),
            _ => |value| value,
        }
    }
}

struct IntegerScalar;

impl ScalarMode for IntegerScalar {
    const DRAG_STEP: f64 = 1.0;
    const DIGITS: usize = 0;
    const INTEGER: bool = true;

    fn clamp(_: ScalarOptions) -> fn(f32) -> f32 {
        |value| value.round()
    }
}

fn scalar_row_for<M: ScalarMode>(
    label: &str,
    value: &TimelineValue<f32>,
    modifier_id: Uuid,
    options: ScalarOptions,
    context: &InspectorContext,
) -> gtk::Widget {
    let context = deferred_context(context);
    scalar_control(
        label,
        value,
        &context,
        ScalarTarget {
            access: ScalarAccess::Modifier {
                id: modifier_id,
                value_id: value.id,
            },
            scope_id: Some(value.id),
            local_time: visual_local_time,
            duration: visual_duration,
            refresh: ProjectChange {
                video: true,
                ..Default::default()
            },
            commit_name: "visual-modifier-value",
        },
        ScalarSpec {
            drag_step: if options.rotating { 1.0 } else { M::DRAG_STEP },
            digits: M::DIGITS,
            integer: M::INTEGER,
            width_chars: 8,
            minimum: options.minimum,
            maximum: options.maximum,
            unit_name: options.unit,
            rotating_icon: options.rotating.then_some(("arrow3-up-symbolic", 0.0)),
            display: |v| v as f64,
            store: |v| v as f32,
            clamp: crate::timeline_value::scalar::ScalarClamp::Function(M::clamp(options)),
        },
    )
}

fn visual_local_time(p: &Project, k: SelectedItem, t: Time) -> Option<Time> {
    crate::video::visual_local_time(p, k, t)
}
fn visual_duration(p: &Project, k: SelectedItem) -> Option<Time> {
    crate::video::visual_duration(p, k)
}

pub(crate) fn number(modifier: &VisualModifier, id: Uuid) -> Option<&TimelineValue<f32>> {
    modifier.number(id)
}
pub(crate) fn number_mut(
    modifier: &mut VisualModifier,
    id: Uuid,
) -> Option<&mut TimelineValue<f32>> {
    modifier.number_mut(id)
}
pub(crate) fn number2(modifier: &VisualModifier, id: Uuid) -> Option<&TimelineValue<glam::Vec2>> {
    modifier.number2(id)
}
pub(crate) fn number2_mut(
    modifier: &mut VisualModifier,
    id: Uuid,
) -> Option<&mut TimelineValue<glam::Vec2>> {
    modifier.number2_mut(id)
}
pub(crate) fn color_mut(
    effect: &mut ModifierEffect,
    id: Uuid,
) -> Option<&mut TimelineValue<shrimply_property_model::Color<u8>>> {
    effect.color_mut(id)
}

pub(crate) fn vec_row(
    label: &str,
    value: &TimelineValue<glam::Vec2>,
    modifier_id: Uuid,
    scale: bool,
    bounds: Option<(f64, f64)>,
    context: &InspectorContext,
) -> gtk::Widget {
    let context = deferred_context(context);
    vec_control(
        label,
        value,
        &context,
        VecTarget {
            access: VecAccess::Modifier {
                id: modifier_id,
                value_id: value.id,
            },
            scope_id: Some(value.id),
            local_time: visual_local_time,
            duration: visual_duration,
            refresh: ProjectChange {
                video: true,
                ..Default::default()
            },
            commit_name: "visual-modifier-vector",
        },
        VecSpec {
            first_prefix: "X",
            second_prefix: "Y",
            drag_step: if scale { 0.01 } else { 1.0 },
            digits: if scale { 2 } else { 0 },
            width_chars: 7,
            minimum: bounds.map(|bounds| bounds.0),
            maximum: bounds.map(|bounds| bounds.1),
            unit_name: if scale { "x" } else { "px" },
        },
    )
}
pub(crate) fn color_row(
    label: &str,
    value: &TimelineValue<shrimply_property_model::Color<u8>>,
    modifier_id: Uuid,
    context: &InspectorContext,
) -> gtk::Widget {
    let context = deferred_context(context);
    color_control(
        label,
        value,
        &context,
        ColorTarget {
            access: ColorAccess::Modifier {
                id: modifier_id,
                value_id: value.id,
            },
            scope_id: Some(value.id),
            local_time: visual_local_time,
            duration: visual_duration,
            refresh: ProjectChange {
                video: true,
                ..Default::default()
            },
            commit_name: "visual-modifier-color",
        },
    )
}

fn deferred_context(context: &InspectorContext) -> InspectorContext {
    let mut context = context.clone();
    let refresh = context.refresh.clone();
    let pending = std::rc::Rc::new(std::cell::Cell::new(false));
    context.refresh = std::rc::Rc::new(move || {
        if pending.replace(true) {
            return;
        }
        let pending = pending.clone();
        let refresh = refresh.clone();
        gtk::glib::idle_add_local_once(move || {
            pending.set(false);
            refresh();
        });
    });
    context
}
