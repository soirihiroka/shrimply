use super::*;
use objc2::sel;
use objc2_app_kit::{NSMenu, NSMenuItem};
use objc2_foundation::{NSPoint, NSString, ns_string};
use shrimply_timeline_core::{
    TrackAddAction, TrackAddMenuEntry, TrackAddSettings, TrackKey,
    draw_state::{TrackButtonId, TrackLabelAction},
    selection_state,
};

impl CanvasView {
    pub(super) fn activate_track_button(
        &self,
        (key, action): TrackButtonId,
        point: glam::Vec2,
    ) -> Result<(), String> {
        match action {
            TrackLabelAction::Add => self.show_track_add_menu(key, point),
            TrackLabelAction::AudioRecord => {
                Err("Microphone recording is not yet connected to the AppKit editor.".into())
            }
            TrackLabelAction::VideoRecord => Err(
                "Screen recording requires a macOS capture backend, which is not yet available."
                    .into(),
            ),
            TrackLabelAction::Toggle | TrackLabelAction::Select => {
                unreachable!("track state actions are handled by the shared scene")
            }
        }
    }

    fn show_track_add_menu(&self, key: TrackKey, point: glam::Vec2) -> Result<(), String> {
        let session = &self.ivars().session;
        let selected = selection_state::selected_tracks(&session.selection_state);
        let import_targets = if selected.contains(&key) {
            selected
        } else {
            vec![key]
        };
        let (address, import_addresses) = {
            let project = session.project.borrow();
            let address =
                selection_state::track_address(&project, key).ok_or("track no longer exists")?;
            let targets = import_targets
                .iter()
                .map(|key| {
                    selection_state::track_address(&project, *key)
                        .ok_or("import track no longer exists")
                })
                .collect::<Result<Vec<_>, _>>()?;
            (address, targets)
        };
        let menu = NSMenu::initWithTitle(NSMenu::alloc(self.mtm()), ns_string!("Add"));
        menu.setAutoenablesItems(false);
        let entries = shrimply_timeline_core::track_add_menu(key.kind);
        for (index, entry) in entries.iter().enumerate() {
            let TrackAddMenuEntry::Action(action) = entry else {
                menu.addItem(&NSMenuItem::separatorItem(self.mtm()));
                continue;
            };
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(self.mtm()),
                    &NSString::from_str(action.label(key.kind)),
                    Some(sel!(chooseTrackAdd:)),
                    ns_string!(""),
                )
            };
            item.setTag(index.try_into().expect("track menu index fits NSInteger"));
            unsafe {
                item.setTarget(Some(self));
            }
            let symbol = match action {
                TrackAddAction::Import => "folder",
                TrackAddAction::Text => "textformat",
                TrackAddAction::Shape => "square.on.circle",
                TrackAddAction::Paint => "paintbrush",
                TrackAddAction::Background => "photo",
                TrackAddAction::Scene3d => "cube",
                TrackAddAction::VideoGeneration => "film",
                TrackAddAction::TextToSpeech => "text.bubble",
                TrackAddAction::AudioGenerator => "waveform",
            };
            item.setImage(Some(&super::super::layout::symbol(
                symbol,
                action.label(key.kind),
            )));
            menu.addItem(&item);
        }
        self.ivars().menu_choice.set(None);
        // Native menus run a nested event loop; hold no scene or project borrow here.
        menu.popUpMenuPositioningItem_atLocation_inView(
            None,
            NSPoint::new(point.x.into(), point.y.into()),
            Some(self),
        );
        let Some(index) = self.ivars().menu_choice.take() else {
            return Ok(());
        };
        let TrackAddMenuEntry::Action(action) = entries[index] else {
            unreachable!("separator cannot activate")
        };
        let (key, import_targets) = {
            let project = session.project.borrow();
            let key = selection_state::track_key(&project, &address)
                .ok_or("track was removed while the menu was open")?;
            let targets = import_addresses
                .iter()
                .map(|address| {
                    selection_state::track_key(&project, address)
                        .ok_or("import track was removed while the menu was open")
                })
                .collect::<Result<Vec<_>, _>>()?;
            (key, targets)
        };
        if action == TrackAddAction::Import {
            return super::super::media::choose_files(
                &self.ivars().imports,
                session,
                &import_targets,
                self.mtm(),
            );
        }
        let preferences = shrimply_state::preferences::snapshot(&session.preferences);
        shrimply_timeline_core::activate_track_add_checked(
            &session.project,
            &session.player_state,
            &session.selection_state,
            key,
            action,
            TrackAddSettings {
                default_visual_duration: preferences.default_visual_duration,
                default_text_font_family: &preferences.default_text_font_family,
            },
        )?;
        Ok(())
    }
}
