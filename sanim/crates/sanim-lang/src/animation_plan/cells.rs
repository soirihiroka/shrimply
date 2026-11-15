//! Scalar and list plans commit through the same compiled window and COW transaction.
use crate::native::Context;
use sanim_core::{Value, math};

pub(crate) fn write_cell(cell: u64, to: Value, context: Context<'_>) -> Result<(), String> {
    let start = *context.cursor;
    let window = math::temporal::Window {
        cycle: context.cycle,
        clip: context.clip,
    };
    let mut next = context.animation.clone();
    if let Value::List(items) = &to
        && next.list_views.has_cell(cell)
    {
        next.write_list_in_window(
            cell,
            start,
            context.duration,
            items.clone(),
            context.easing,
            window,
        )?;
    }
    next.reactive.write_cell_in_window(
        cell,
        start,
        context.duration,
        to,
        context.easing,
        window,
    )?;
    if let Some(interval) = window.interval(start, context.duration)? {
        next.duration = next.duration.max(interval.end);
    }
    *context.animation = next;
    Ok(())
}
