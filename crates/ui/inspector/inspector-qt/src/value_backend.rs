use crate::item::InspectorAction;
use crate::section::InspectorControl;

pub(crate) fn fraction_value(control: &InspectorControl) -> f64 {
    let numerator = control
        .components
        .first()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or_default();
    let denominator = control
        .components
        .get(1)
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| *value != 0.0)
        .unwrap_or(1.0);
    numerator / denominator
}

pub(crate) fn boolean_action(action: InspectorAction, active: bool) -> InspectorAction {
    match action {
        InspectorAction::SetBoolean { path, .. } => InspectorAction::SetBoolean {
            path,
            value: active,
        },
        InspectorAction::SetAlphaMask { target, .. } => InspectorAction::SetAlphaMask {
            target,
            enabled: active,
        },
        action => action,
    }
}
