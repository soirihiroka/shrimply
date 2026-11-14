//! One cached Taffy flex layout boundary, shared by rendering and verification.
use crate::{Align, Direction, Element, Justify, Layout, math::to_f64};
use std::{cell::RefCell, collections::BTreeMap, sync::Arc};
use taffy::prelude::*;

const MAX_CACHED_LAYOUTS: usize = 64;
const MAX_CACHED_ITEMS: usize = 32_768;
type SizeKey = (u64, u64, u64, u64, Option<(u64, bool)>, crate::PositionMode);
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    width: u64,
    height: Option<u64>,
    gap: u64,
    padding: u64,
    direction: Direction,
    justify: Justify,
    align: Align,
    wrap: bool,
    sizes: Vec<SizeKey>,
}
#[derive(Clone, Copy)]
pub(super) struct Item {
    pub position: [f64; 2],
    pub size: [f64; 2],
}
pub(super) struct Plan {
    pub items: Arc<[Item]>,
    pub size: [f64; 2],
}
type Positions = Arc<Plan>;
thread_local! { static CACHE:RefCell<BTreeMap<Key,Positions>>=const{RefCell::new(BTreeMap::new())}; }

pub(super) fn positions(
    children: &imbl::Vector<Element>,
    style: &Layout,
    width: f64,
    height: Option<f64>,
    direction: Direction,
) -> Result<Positions, String> {
    let sizes = children
        .iter()
        .map(|child| {
            if child.layout.position_mode == crate::PositionMode::Absolute {
                // Its paint is resolved independently; none of its dimensions
                // may invalidate the sibling flow plan.
                Ok((0., 0.))
            } else {
                super::extent(child)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let gap = to_f64(style.gap)?;
    let padding = to_f64(style.padding)?;
    let key = Key {
        width: width.to_bits(),
        height: height.map(f64::to_bits),
        gap: gap.to_bits(),
        padding: padding.to_bits(),
        direction,
        justify: style.justify,
        align: style.align,
        wrap: style.wrap,
        sizes: sizes
            .iter()
            .zip(children.iter())
            .map(|((w, h), child)| -> Result<SizeKey, String> {
                let grow = to_f64(child.layout.flex_grow)?;
                let shrink = to_f64(child.layout.flex_shrink)?;
                if grow < 0. || shrink < 0. {
                    return Err("flex factors must be nonnegative".into());
                }
                let basis = child
                    .layout
                    .flex_basis
                    .map(|basis| -> Result<_, String> {
                        let value = to_f64(basis.value)?;
                        if value < 0.
                            || !matches!(basis.unit, crate::Unit::Length | crate::Unit::Percent)
                        {
                            return Err(
                                "flex_basis requires nonnegative points or a percentage".into()
                            );
                        }
                        Ok((value.to_bits(), basis.unit == crate::Unit::Percent))
                    })
                    .transpose()?;
                let flex =
                    style.flex && child.layout.position_mode != crate::PositionMode::Absolute;
                Ok((
                    w.to_bits(),
                    h.to_bits(),
                    if flex { grow.to_bits() } else { 0 },
                    if flex { shrink.to_bits() } else { 0 },
                    if flex { basis } else { None },
                    child.layout.position_mode,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?,
    };
    if let Some(cached) = CACHE.with(|cache| cache.borrow().get(&key).cloned()) {
        return Ok(cached);
    }
    let coordinate = |value: f64| -> Result<f32, String> {
        let converted = value as f32;
        if !converted.is_finite() || value != 0. && converted == 0. {
            Err("layout dimension exceeds Taffy numeric range".into())
        } else {
            Ok(converted)
        }
    };
    let mut tree: TaffyTree<()> = TaffyTree::with_capacity(children.len() + 1);
    tree.disable_rounding();
    let nodes = sizes
        .iter()
        .zip(children.iter())
        .map(|((w, h), child)| {
            let flex = style.flex && child.layout.position_mode != crate::PositionMode::Absolute;
            tree.new_leaf(Style {
                position: if child.layout.position_mode == crate::PositionMode::Absolute {
                    Position::Absolute
                } else {
                    Position::Relative
                },
                size: Size {
                    width: length(coordinate(*w)?),
                    height: length(coordinate(*h)?),
                },
                flex_grow: if flex {
                    coordinate(to_f64(child.layout.flex_grow)?)?
                } else {
                    0.
                },
                flex_shrink: if flex {
                    coordinate(to_f64(child.layout.flex_shrink)?)?
                } else {
                    0.
                },
                flex_basis: if flex {
                    child
                        .layout
                        .flex_basis
                        .map(|basis| -> Result<Dimension, String> {
                            let value = coordinate(to_f64(basis.value)?)?;
                            match basis.unit {
                                crate::Unit::Length => Ok(length(value)),
                                crate::Unit::Percent => Ok(percent(value)),
                                _ => Err("flex_basis requires points or a percentage".into()),
                            }
                        })
                        .transpose()?
                        .unwrap_or(Dimension::AUTO)
                } else {
                    Dimension::AUTO
                },
                ..Style::default()
            })
            .map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>, String>>()?;
    let justify_content = match style.justify {
        Justify::Start => JustifyContent::START,
        Justify::Center => JustifyContent::CENTER,
        Justify::End => JustifyContent::END,
        Justify::SpaceBetween => JustifyContent::SPACE_BETWEEN,
        Justify::SpaceAround => JustifyContent::SPACE_AROUND,
        Justify::SpaceEvenly => JustifyContent::SPACE_EVENLY,
    };
    let align_items = match style.align {
        Align::Start => AlignItems::START,
        Align::Center => AlignItems::CENTER,
        Align::End => AlignItems::END,
    };
    let root = tree
        .new_with_children(
            Style {
                display: Display::Flex,
                size: Size {
                    width: length(coordinate(width)?),
                    height: height
                        .map(|height| coordinate(height).map(length))
                        .transpose()?
                        .unwrap_or(Dimension::AUTO),
                },
                flex_direction: if direction == Direction::Row {
                    FlexDirection::Row
                } else {
                    FlexDirection::Column
                },
                flex_wrap: if style.wrap {
                    FlexWrap::Wrap
                } else {
                    FlexWrap::NoWrap
                },
                gap: Size {
                    width: length(coordinate(gap)?),
                    height: length(coordinate(gap)?),
                },
                padding: taffy::Rect {
                    left: length(coordinate(padding)?),
                    right: length(coordinate(padding)?),
                    top: length(coordinate(padding)?),
                    bottom: length(coordinate(padding)?),
                },
                justify_content,
                align_items,
                align_content: AlignContent::START,
                ..Style::default()
            },
            &nodes,
        )
        .map_err(|e| e.to_string())?;
    tree.compute_layout(root, Size::MAX_CONTENT)
        .map_err(|e| e.to_string())?;
    let root_layout = tree.layout(root).map_err(|error| error.to_string())?;
    let root_size = [
        f64::from(root_layout.size.width),
        f64::from(root_layout.size.height),
    ];
    let items = nodes
        .iter()
        .map(|node| {
            let layout = tree.layout(*node).map_err(|error| error.to_string())?;
            let position = [f64::from(layout.location.x), f64::from(layout.location.y)];
            let size = [f64::from(layout.size.width), f64::from(layout.size.height)];
            if position.iter().chain(size.iter()).any(|v| !v.is_finite()) {
                return Err("Taffy layout overflow".to_string());
            }
            Ok(Item { position, size })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let positions = Arc::new(Plan {
        items: items.into(),
        size: root_size,
    });
    if children.len() <= MAX_CACHED_ITEMS {
        CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.len() >= MAX_CACHED_LAYOUTS
                || cache.values().map(|v| v.items.len()).sum::<usize>() + children.len()
                    > MAX_CACHED_ITEMS
            {
                cache.clear();
            }
            cache.insert(key, positions.clone());
        });
    }
    Ok(positions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_geometry_and_flex_factors_reuse_the_same_flow_plan() {
        let mut children: imbl::Vector<_> = [Element {
            id: "overlay".into(),
            geometry: Arc::new(crate::Geometry::Circle { radius: 10.into() }),
            position: crate::Vec2::zero(crate::Unit::Length),
            rotation: 0.into(),
            scale: 1.into(),
            opacity: 1.into(),
            fill: crate::Color::parse("blue3").unwrap().into(),
            stroke: None,
            reveal: Default::default(),
            horizontal: Default::default(),
            layout: Layout {
                position_mode: crate::PositionMode::Absolute,
                ..Default::default()
            },
        }]
        .into_iter()
        .collect();
        let style = Layout {
            flex: true,
            ..Default::default()
        };
        let first = positions(&children, &style, 100., None, Direction::Column).unwrap();
        let overlay = children.front_mut().unwrap();
        overlay.geometry = Arc::new(crate::Geometry::Circle { radius: 20.into() });
        overlay.layout.flex_grow = 2.into();
        overlay.layout.flex_shrink = 3.into();
        overlay.layout.flex_basis = Some(crate::Number {
            value: 80.into(),
            unit: crate::Unit::Length,
        });
        let resized = positions(&children, &style, 100., None, Direction::Column).unwrap();
        assert!(Arc::ptr_eq(&first, &resized));
        children.front_mut().unwrap().layout.position_mode = crate::PositionMode::Relative;
        let flowed = positions(&children, &style, 100., None, Direction::Column).unwrap();
        assert!(!Arc::ptr_eq(&first, &flowed));
        assert_eq!(first.size[1], 0.);
        assert!(flowed.size[1] > 0.);
    }
}
