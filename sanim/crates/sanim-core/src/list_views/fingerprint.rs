//! Deterministic visual keys retain edit structure instead of flattening every
//! shared snapshot into a repeated copy of the complete item geometry.
use super::*;
use serde::{
    Serializer,
    ser::{SerializeMap, SerializeSeq},
};

struct CatalogEntry<'a>(&'a Element);
#[derive(Serialize)]
enum GeometryKey<'a> {
    Leaf(&'a Geometry),
    Group(Vec<&'a str>),
    Rectangle(
        &'a Fraction,
        &'a Fraction,
        &'a crate::CornerRadius,
        Vec<&'a str>,
    ),
}
impl Serialize for CatalogEntry<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Exhaustive destructuring makes new visual Element fields a compile
        // error here until they are included in the cache identity.
        let Element {
            id,
            geometry,
            position,
            rotation,
            scale,
            opacity,
            fill,
            stroke,
            reveal,
            horizontal,
            layout,
        } = self.0;
        let geometry = match geometry.as_ref() {
            Geometry::Group { children } => {
                GeometryKey::Group(children.iter().map(|child| child.id.as_str()).collect())
            }
            Geometry::Rectangle {
                width,
                height,
                radius,
                children,
            } => GeometryKey::Rectangle(
                width,
                height,
                radius,
                children.iter().map(|child| child.id.as_str()).collect(),
            ),
            geometry => GeometryKey::Leaf(geometry),
        };
        (
            id, geometry, position, rotation, scale, opacity, fill, stroke, reveal, horizontal,
            layout,
        )
            .serialize(serializer)
    }
}

fn ids(items: &List) -> Result<Vec<&str>, String> {
    Ok(elements(items)?
        .map(|element| element.id.as_str())
        .collect())
}
fn descriptor(edit: &crate::ListEdit) -> (&'static str, usize, usize, Option<&str>) {
    fn identity(value: &Value) -> Option<&str> {
        match value {
            Value::Element(element) => Some(element.id.as_str()),
            _ => None,
        }
    }
    match edit {
        crate::ListEdit::Append(value) => ("append", 0, 0, identity(value)),
        crate::ListEdit::Insert { index, value } => ("insert", *index, 0, identity(value)),
        crate::ListEdit::RemoveAt(index) => ("remove_at", *index, 0, None),
        crate::ListEdit::ReplaceAt { index, value } => ("replace_at", *index, 0, identity(value)),
        crate::ListEdit::Move {
            from_index,
            to_index,
        } => ("move_item", *from_index, *to_index, None),
    }
}
struct Catalog<'a> {
    lists: &'a ListViews,
    cells: &'a BTreeSet<u64>,
}
impl Serialize for Catalog<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        for (id, element) in &self.lists.catalog {
            if self
                .lists
                .owners
                .get(id)
                .is_some_and(|cell| self.cells.contains(cell))
            {
                map.serialize_entry(id, &CatalogEntry(element))?;
            }
        }
        map.end()
    }
}
struct Transitions<'a>(&'a Vector<Transition>);
impl Serialize for Transitions<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for event in self.0 {
            if let Some(operation) = &event.operation {
                sequence.serialize_element(&(
                    event.start,
                    event.end,
                    event.easing,
                    Some(descriptor(operation)),
                    Option::<Vec<&str>>::None,
                ))?;
            } else {
                let items = ids(&event.to.items).map_err(serde::ser::Error::custom)?;
                sequence.serialize_element(&(
                    event.start,
                    event.end,
                    event.easing,
                    Option::<(&str, usize, usize, Option<&str>)>::None,
                    Some(items),
                ))?;
            }
        }
        sequence.end()
    }
}
struct Channels<'a> {
    lists: &'a ListViews,
    cells: &'a BTreeSet<u64>,
}
impl Serialize for Channels<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.cells.len()))?;
        for cell in self.cells {
            let channel =
                self.lists.channels.get(cell).ok_or_else(|| {
                    serde::ser::Error::custom("live view references missing channel")
                })?;
            let initial = ids(&channel.initial.items).map_err(serde::ser::Error::custom)?;
            map.serialize_entry(cell, &(initial, Transitions(&channel.transitions)))?;
        }
        map.end()
    }
}

impl Animation {
    pub fn visual_key(&self) -> Result<Vec<u8>, String> {
        let visible = self
            .list_views
            .reachable_views(self.layers.iter().map(|layer| &layer.element));
        let views: std::collections::BTreeMap<_, _> = visible
            .iter()
            .map(|id| (*id, self.list_views.views[*id]))
            .collect();
        let cells = views.values().copied().collect();
        let Animation {
            dpi,
            width,
            height,
            rate,
            duration,
            background,
            layers,
            tracks,
            reactive,
            list_views,
            element_index,
            flow_controlled,
        } = self;
        serde_json::to_vec(&(
            crate::SEMANTICS_VERSION,
            dpi,
            width,
            height,
            rate,
            duration,
            background,
            layers,
            tracks,
            reactive,
            (
                &views,
                Catalog {
                    lists: list_views,
                    cells: &cells,
                },
                Channels {
                    lists: list_views,
                    cells: &cells,
                },
            ),
            element_index,
            flow_controlled,
        ))
        .map_err(|error| error.to_string())
    }
}
