//! Deterministic visual keys retain edit structure instead of flattening every
//! shared snapshot into a repeated copy of the complete item geometry.
use super::*;
use serde::{
    Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use std::sync::Arc;

#[derive(Default)]
struct Signatures {
    documents: std::cell::RefCell<std::collections::BTreeMap<usize, String>>,
    options: std::cell::RefCell<std::collections::BTreeMap<usize, String>>,
    fonts: std::cell::RefCell<std::collections::BTreeMap<usize, String>>,
}
#[derive(Serialize)]
enum FontKey {
    Asset(String),
    Family(Arc<[String]>),
}
impl Signatures {
    fn font(&self, font: &crate::Font) -> FontKey {
        match font {
            crate::Font::Family(names) => FontKey::Family(names.clone()),
            crate::Font::Asset(bytes) => FontKey::Asset(
                self.fonts
                    .borrow_mut()
                    .entry(Arc::as_ptr(bytes) as usize)
                    .or_insert_with(|| crate::digest(bytes))
                    .clone(),
            ),
        }
    }
    fn document(&self, document: &Arc<crate::code::Document>) -> Result<String, String> {
        let id = Arc::as_ptr(document) as usize;
        if let Some(hash) = self.documents.borrow().get(&id) {
            return Ok(hash.clone());
        }
        let hash = crate::digest(
            &serde_json::to_vec(&document.visual_recipe()).map_err(|e| e.to_string())?,
        );
        self.documents.borrow_mut().insert(id, hash.clone());
        Ok(hash)
    }
    fn options(&self, options: &Arc<crate::code::view::Options>) -> Result<String, String> {
        let id = Arc::as_ptr(options) as usize;
        if let Some(hash) = self.options.borrow().get(&id) {
            return Ok(hash.clone());
        }
        let hash = crate::digest(
            &serde_json::to_vec(&options.visual_recipe(self.font(&options.font)))
                .map_err(|e| e.to_string())?,
        );
        self.options.borrow_mut().insert(id, hash.clone());
        Ok(hash)
    }
    fn code(
        &self,
        document: &Arc<crate::code::Document>,
        options: &Arc<crate::code::view::Options>,
    ) -> Result<String, String> {
        Ok(crate::digest(
            &serde_json::to_vec(&(self.document(document)?, self.options(options)?))
                .map_err(|e| e.to_string())?,
        ))
    }
    fn snapshot(&self, snapshot: &crate::code::view::Snapshot) -> Result<String, String> {
        Ok(crate::digest(
            &serde_json::to_vec(&(
                self.code(&snapshot.document, &snapshot.options)?,
                &snapshot.folded,
                &snapshot.selection,
            ))
            .map_err(|e| e.to_string())?,
        ))
    }
}
struct CatalogEntry<'a>(&'a Element, &'a Signatures);
struct TextKey<'a>(&'a Geometry, &'a Signatures);
impl Serialize for TextKey<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let Geometry::Text {
            text,
            size,
            outline,
            bounds,
            font,
            pending,
        } = self.0
        else {
            return Err(serde::ser::Error::custom(
                "Text visual key requires Text geometry",
            ));
        };
        // A font recipe can affect later text signals; hash assets once rather than serializing them per glyph.
        (text, size, outline, bounds, pending, self.1.font(font)).serialize(serializer)
    }
}
#[derive(Serialize)]
enum GeometryKey<'a> {
    Leaf(&'a Geometry),
    Text(TextKey<'a>),
    Group(Vec<&'a str>),
    Code(Vec<&'a str>, String),
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
            Geometry::Code {
                children,
                document,
                options,
                ..
            } => GeometryKey::Code(
                children.iter().map(|child| child.id.as_str()).collect(),
                self.1
                    .code(document, options)
                    .map_err(serde::ser::Error::custom)?,
            ),
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
            geometry @ Geometry::Text { .. } => GeometryKey::Text(TextKey(geometry, self.1)),
            geometry => GeometryKey::Leaf(geometry),
        };
        (
            id, geometry, position, rotation, scale, opacity, fill, stroke, reveal, horizontal,
            layout,
        )
            .serialize(serializer)
    }
}

/// Source trees/style recipes are preparation metadata. Only the native visual
/// children and normal element properties identify a drawable CodeView frame.
struct VisualElement<'a>(&'a Element, &'a Signatures);
#[derive(Serialize)]
enum VisualGeometry<'a> {
    Leaf(&'a Geometry),
    Text(TextKey<'a>),
    Group(Vec<VisualElement<'a>>),
    Code(Vec<VisualElement<'a>>, String),
    Rectangle(
        &'a Fraction,
        &'a Fraction,
        &'a crate::CornerRadius,
        Vec<VisualElement<'a>>,
    ),
}
impl Serialize for VisualElement<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
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
            Geometry::Group { children } => VisualGeometry::Group(
                children
                    .iter()
                    .map(|child| VisualElement(child, self.1))
                    .collect(),
            ),
            Geometry::Code {
                children,
                document,
                options,
                ..
            } => VisualGeometry::Code(
                children
                    .iter()
                    .map(|child| VisualElement(child, self.1))
                    .collect(),
                self.1
                    .code(document, options)
                    .map_err(serde::ser::Error::custom)?,
            ),
            Geometry::Rectangle {
                width,
                height,
                radius,
                children,
            } => VisualGeometry::Rectangle(
                width,
                height,
                radius,
                children
                    .iter()
                    .map(|child| VisualElement(child, self.1))
                    .collect(),
            ),
            geometry @ Geometry::Text { .. } => VisualGeometry::Text(TextKey(geometry, self.1)),
            geometry => VisualGeometry::Leaf(geometry),
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
    signatures: &'a Signatures,
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
                map.serialize_entry(id, &CatalogEntry(element, self.signatures))?;
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

struct CodeEvents<'a>(&'a Animation, &'a Signatures);
impl Serialize for CodeEvents<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        for (id, state) in &self.0.code_views {
            if self.0.find(id).is_none() || state.events.is_empty() {
                continue;
            }
            let events: Result<Vec<_>, String> = state
                .events
                .values()
                .map(|event| {
                    Ok((
                        event.start,
                        event.end,
                        event.easing,
                        event.animation,
                        self.1.snapshot(&event.from)?,
                        self.1.snapshot(&event.to)?,
                    ))
                })
                .collect();
            map.serialize_entry(id, &events.map_err(serde::ser::Error::custom)?)?;
        }
        map.end()
    }
}

impl Animation {
    pub fn visual_key(&self) -> Result<Vec<u8>, String> {
        let signatures = Signatures::default();
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
            code_views: _,
        } = self;
        serde_json::to_vec(&(
            crate::SEMANTICS_VERSION,
            dpi,
            width,
            height,
            rate,
            duration,
            background,
            layers
                .iter()
                .map(|layer| (&layer.birth, VisualElement(&layer.element, &signatures)))
                .collect::<Vec<_>>(),
            tracks,
            reactive,
            (
                &views,
                Catalog {
                    lists: list_views,
                    cells: &cells,
                    signatures: &signatures,
                },
                Channels {
                    lists: list_views,
                    cells: &cells,
                },
            ),
            element_index,
            flow_controlled,
            CodeEvents(self, &signatures),
        ))
        .map_err(|error| error.to_string())
    }
}
