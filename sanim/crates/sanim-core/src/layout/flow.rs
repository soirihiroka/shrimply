//! One cached Taffy flex layout boundary, shared by rendering and verification.
use crate::{Align, Direction, Element, Justify, Layout, math::to_f64};
use std::{cell::RefCell, collections::BTreeMap, sync::Arc};
use taffy::prelude::*;

const MAX_CACHED_LAYOUTS: usize = 64;
const MAX_CACHED_ITEMS: usize = 32_768;
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    width: u64,
    height: u64,
    gap: u64,
    padding: u64,
    direction: Direction,
    justify: Justify,
    align: Align,
    wrap: bool,
    sizes: Vec<(u64, u64)>,
}
type Positions = Arc<[[f64; 2]]>;
thread_local! { static CACHE:RefCell<BTreeMap<Key,Positions>>=const{RefCell::new(BTreeMap::new())}; }

pub(super) fn positions(
    children: &[Element],
    style: &Layout,
    width: f64,
    height: f64,
    direction: Direction,
) -> Result<Positions, String> {
    let sizes = children
        .iter()
        .map(super::extent)
        .collect::<Result<Vec<_>, _>>()?;
    let gap = to_f64(style.gap)?;
    let padding = to_f64(style.padding)?;
    let key = Key {
        width: width.to_bits(),
        height: height.to_bits(),
        gap: gap.to_bits(),
        padding: padding.to_bits(),
        direction,
        justify: style.justify,
        align: style.align,
        wrap: style.wrap,
        sizes: sizes
            .iter()
            .map(|(w, h)| (w.to_bits(), h.to_bits()))
            .collect(),
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
        .map(|(w, h)| {
            tree.new_leaf(Style {
                size: Size {
                    width: length(coordinate(*w)?),
                    height: length(coordinate(*h)?),
                },
                flex_shrink: 0.,
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
                    height: length(coordinate(height)?),
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
    let positions: Positions = nodes
        .iter()
        .map(|node| {
            let layout = tree.layout(*node).map_err(|e| e.to_string())?;
            let position = [f64::from(layout.location.x), f64::from(layout.location.y)];
            if position.iter().any(|v| !v.is_finite()) {
                return Err("Taffy layout overflow".into());
            }
            Ok(position)
        })
        .collect::<Result<Vec<_>, String>>()?
        .into();
    if children.len() <= MAX_CACHED_ITEMS {
        CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.len() >= MAX_CACHED_LAYOUTS
                || cache.values().map(|v| v.len()).sum::<usize>() + children.len()
                    > MAX_CACHED_ITEMS
            {
                cache.clear();
            }
            cache.insert(key, positions.clone());
        });
    }
    Ok(positions)
}
