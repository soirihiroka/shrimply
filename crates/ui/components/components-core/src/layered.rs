use std::{cell::Cell, rc::Rc};

pub const KEYFRAMES_ICON: &str = "stopwatch-symbolic";
pub const EXPRESSION_ICON: &str = "code-symbolic";
pub const REVEAL_DURATION_MS: u32 = 180;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayeredEdit<T> {
    Base(T),
    Keyframe(T),
}

pub fn component_changes<T: Copy + PartialEq, const N: usize>(
    previous: [T; N],
    next: [T; N],
) -> Vec<(usize, T)> {
    previous
        .into_iter()
        .zip(next)
        .enumerate()
        .filter_map(|(component, (previous, next))| (previous != next).then_some((component, next)))
        .collect()
}

#[derive(Clone, Default)]
pub struct LayeredPropertyController {
    keyframes: Rc<Cell<bool>>,
    expression: Rc<Cell<bool>>,
    active_component: Rc<Cell<usize>>,
}

impl LayeredPropertyController {
    pub fn set_keyframes(&self, enabled: bool) {
        self.keyframes.set(enabled);
    }

    pub fn set_expression(&self, enabled: bool) {
        self.expression.set(enabled);
    }

    pub fn keyframes(&self) -> bool {
        self.keyframes.get()
    }

    pub fn expression(&self) -> bool {
        self.expression.get()
    }

    pub fn active_component(&self) -> usize {
        self.active_component.get()
    }

    pub fn select_component<const N: usize>(&self, component: usize) {
        assert!(component < N, "layered value component is out of bounds");
        self.active_component.set(component);
    }

    pub fn edit<T>(&self, value: T) -> LayeredEdit<T> {
        if self.keyframes.get() {
            LayeredEdit::Keyframe(value)
        } else {
            LayeredEdit::Base(value)
        }
    }

    pub fn edit_component_value<T, const N: usize>(
        &self,
        component: usize,
        value: T,
    ) -> LayeredEdit<T> {
        self.select_component::<N>(component);
        self.edit(value)
    }

    pub fn edit_component<const N: usize>(
        &self,
        value: [f64; N],
        component: usize,
    ) -> LayeredEdit<([f64; N], f64)> {
        let component_value = *value.get(component).expect("layered value component");
        self.select_component::<N>(component);
        self.edit((value, component_value))
    }
}
