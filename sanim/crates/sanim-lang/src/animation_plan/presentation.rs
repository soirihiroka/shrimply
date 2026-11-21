use super::{Flow, Node, TimeDimension, layout, lower_layout, math, plan};
use crate::{Diagnostic, Prepared, layout_diagnostic};
use sanim_core::{Animation, Fraction};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct PageStop {
    pub at: Fraction,
    pub resume: Fraction,
}

pub struct Presentation {
    pub animation: Animation,
    /// Manual stops followed by the implicit final frame when the deck has a tail.
    pub stops: Vec<PageStop>,
}

impl Prepared {
    /// Lower the same source for interactive playback, giving static pages distinct times.
    /// Source-time contracts have already been checked on the ordinary timeline.
    pub fn presentation(&self) -> Result<Presentation, Diagnostic> {
        prepare(self).map_err(|error| layout_diagnostic(error, &self.plan, &self.source))
    }
}

fn prepare(prepared: &Prepared) -> Result<Presentation, plan::LayoutError> {
    if prepared.animation.indefinite {
        return Err("Sanim Presenter needs a finite deck".into());
    }
    let frame = math::arithmetic("/", 1.into(), prepared.animation.rate)?;
    let reserved = TimeDimension::Fixed(frame);
    let mut root = prepared.plan.as_ref().clone();
    let ends_with_page = if matches!(root.node.as_ref(), Node::Page) {
        root.style.duration = reserved;
        true
    } else if root.style.flow == Flow::Sequence {
        match Arc::make_mut(&mut root.node) {
            Node::Container(children) | Node::Scope { children, .. } => {
                for child in children.iter_mut() {
                    if matches!(child.node.as_ref(), Node::Page) {
                        Arc::make_mut(child).style.duration = reserved;
                    }
                }
                children
                    .back()
                    .is_some_and(|child| matches!(child.node.as_ref(), Node::Page))
            }
            _ => false,
        }
    } else {
        false
    };

    let layout = layout(&root, None, &prepared.plan_limits)?;
    let end = layout
        .duration
        .ok_or("Sanim Presenter needs a finite deck")?;
    let mut stops = Vec::new();
    for placement in &layout.placements {
        if !matches!(placement.animation.node.as_ref(), Node::Page) {
            continue;
        }
        let at = placement.start;
        let resume = math::temporal::end(at, frame)?;
        let message = if placement.animation.style.duration != reserved {
            Some("page() must be in the outer sequence of the presentation")
        } else if resume > end || placement.clip.is_some_and(|(a, b)| at < a || resume > b) {
            Some("page() is clipped by the outer sequence's duration")
        } else {
            None
        };
        if let Some(message) = message {
            return Err(match &placement.animation.origin {
                Some(origin) => plan::LayoutError::InvalidAt {
                    location: origin.location.clone(),
                    message: message.into(),
                },
                None => message.into(),
            });
        }
        stops.push(PageStop { at, resume });
    }
    let animation = if stops.is_empty() && prepared.animation.duration == end {
        prepared.animation.clone()
    } else {
        lower_layout(layout, &prepared.plan_base, &prepared.plan_limits)?.animation
    };
    if !ends_with_page || stops.last().is_some_and(|stop| stop.resume < end) {
        stops.push(PageStop {
            at: end,
            resume: end,
        });
    }
    Ok(Presentation { animation, stops })
}
