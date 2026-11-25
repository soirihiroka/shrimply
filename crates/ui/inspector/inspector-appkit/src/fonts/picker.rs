use super::super::control::Context;
use objc2_app_kit::NSWindow;
use objc2_foundation::MainThreadMarker;
use shrimply_components_appkit::{FontPicker, FontPickerItem};
use shrimply_inspector_core::{
    InspectorControl,
    font_cache::{self, FontFamily, FontSource, GoogleFamily},
    font_selector::{Browser, FamilyEdit},
};
use shrimply_project_document::project::FontFamily as ProjectFontFamily;
use skia_safe::{FontMgr, FontStyle};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const GOOGLE_LOOKUP_DELAY: Duration = Duration::from_millis(500);
type Sources = Arc<Mutex<HashMap<String, (FontFamily, Option<GoogleFamily>)>>>;

pub(super) fn show(
    parent: &NSWindow,
    selected: Option<(usize, ProjectFontFamily)>,
    control: &InspectorControl,
    context: &Context,
    mtm: MainThreadMarker,
) {
    let browser = Rc::new(RefCell::new(Browser::default()));
    browser.borrow_mut().open();
    let sources = Sources::default();
    let deadline = Rc::new(Cell::new(None::<(Instant, u64)>));
    let apply = Rc::new({
        let controller = context.controller.clone();
        let target = context.target.clone();
        let dirty = context.dirty.clone();
        let control = control.clone();
        let index = selected.as_ref().map(|(index, _)| *index);
        move |picker: &FontPicker, family: FontFamily| {
            let family = font_cache::project_family(&family);
            let edit = match index {
                Some(index) => FamilyEdit::Replace { index, family },
                None => FamilyEdit::Append(family),
            };
            match controller.edit_control_font_family(&target, &control, edit) {
                Ok(()) => {
                    dirty.set(true);
                    picker.close();
                }
                Err(error) => {
                    picker.set_busy(false);
                    picker.set_status(&error);
                }
            }
        }
    });
    let loader_sources = sources.clone();
    FontPicker::builder(move |item| {
        let (family, lookup) = loader_sources
            .lock()
            .expect("font preview sources lock")
            .get(&item.id)
            .cloned()
            .ok_or_else(|| "Font left the search results".to_string())?;
        match family.source {
            FontSource::Local => {
                thread_local! { static MANAGER: FontMgr = FontMgr::new(); }
                MANAGER
                    .with(|manager| manager.match_family_style(&family.name, FontStyle::default()))
                    .ok_or_else(|| format!("Font {} is unavailable", family.name))
            }
            FontSource::Google if family.revision < 0 => font_cache::preview_google_family(
                &lookup.ok_or("Google font metadata is unavailable")?,
            ),
            FontSource::Google => font_cache::preview_typeface(&family.name),
        }
    })
    .on_search({
        let browser = browser.clone();
        let sources = sources.clone();
        let deadline = deadline.clone();
        let selected = selected.clone();
        move |picker, query, immediate| {
            let mut browser = browser.borrow_mut();
            let generation = browser.set_query(query);
            if immediate {
                browser.begin_lookup(generation);
                deadline.set(None);
            } else {
                deadline.set(Some((Instant::now() + GOOGLE_LOOKUP_DELAY, generation)));
            }
            refresh(
                picker,
                &browser,
                &sources,
                selected.as_ref().map(|(_, family)| family),
            );
        }
    })
    .on_choose({
        let browser = browser.clone();
        let sources = sources.clone();
        let apply = apply.clone();
        move |picker, item| {
            let family = sources
                .lock()
                .expect("font preview sources lock")
                .get(&item.id)
                .map(|(family, _)| family.clone());
            let Some(family) = family else {
                return;
            };
            if family.source == FontSource::Local {
                apply(picker, family);
                return;
            }
            let result = browser.borrow_mut().activate(family);
            match result {
                Ok(()) => {
                    picker.set_busy(true);
                    picker.set_status(browser.borrow().status());
                }
                Err(error) => picker.set_status(&error),
            }
        }
    })
    .on_poll(move |picker| {
        if let Some((when, generation)) = deadline.get()
            && Instant::now() >= when
        {
            deadline.set(None);
            browser.borrow_mut().begin_lookup(generation);
            picker.set_status(browser.borrow().status());
        }
        let update = browser.borrow_mut().poll();
        if update.visible_changed {
            refresh(
                picker,
                &browser.borrow(),
                &sources,
                selected.as_ref().map(|(_, family)| family),
            );
        } else if update.changed {
            picker.set_status(browser.borrow().status());
        }
        for result in update.activations {
            match result {
                Ok(family) => apply(picker, family),
                Err(error) => {
                    picker.set_busy(false);
                    picker.set_status(&error);
                }
            }
        }
    })
    .show(parent, mtm)
    .set_status("Loading fonts…");
}

fn refresh(
    picker: &FontPicker,
    browser: &Browser,
    sources: &Sources,
    selected: Option<&ProjectFontFamily>,
) {
    let mut registry = sources.lock().expect("font preview sources lock");
    registry.clear();
    let mut selected_id = None;
    let items = browser
        .visible()
        .iter()
        .map(|family| {
            let id = format!(
                "{:?}:{}:{}",
                family.source,
                family.revision,
                family.name.to_lowercase()
            );
            if selected.is_some_and(|selected| {
                selected.name().eq_ignore_ascii_case(&family.name)
                    && matches!(selected, ProjectFontFamily::GoogleFonts { .. })
                        == (family.source == FontSource::Google)
            }) {
                selected_id = Some(id.clone());
            }
            registry.insert(id.clone(), (family.clone(), browser.lookup().cloned()));
            FontPickerItem {
                id,
                name: family.name.clone(),
                google: family.source == FontSource::Google,
            }
        })
        .collect();
    drop(registry);
    picker.set_items(items);
    picker.set_selected(selected_id);
    picker.set_status(
        if browser.visible().is_empty() && browser.status().is_empty() {
            "No matching fonts"
        } else {
            browser.status()
        },
    );
}
