use super::*;
use objc2::sel;
use objc2_app_kit::{NSMenu, NSMenuItem};
use objc2_foundation::{NSString, ns_string};
use shrimply_editor_state::player_state;
use shrimply_timeline_skia::{
    TrackAddAction, TrackAddMenuEntry, TrackAddSettings, scene::TrackAddMenuRequest,
    selection_state,
};

impl CanvasView {
    pub(super) fn handle_timeline_requests(&self) -> Result<(), String> {
        let requests = {
            let mut content = self.ivars().content.borrow_mut();
            let Content::Timeline(scene) = &mut *content else {
                return Ok(());
            };
            scene.take_requests()
        };
        if requests.pause_playback {
            player_state::set_playing(&self.ivars().session.player_state, false);
        }
        if let Some((point, touch)) = requests.context_menu {
            self.open_timeline_context_menu(point, touch);
        }
        if let Some(request) = requests.track_add {
            self.show_track_add_menu(request)?;
        }
        if let Some(request) = requests.audio_record {
            let mut content = self.ivars().content.borrow_mut();
            let Content::Timeline(scene) = &mut *content else {
                return Err("Microphone recording requires the timeline".into());
            };
            scene.toggle_audio_recording_at(request.key, request.position)?;
        }
        if let Some(request) = requests.video_record {
            {
                let mut content = self.ivars().content.borrow_mut();
                let Content::Timeline(scene) = &mut *content else {
                    return Err("Video recording requires the timeline".into());
                };
                scene.toggle_video_recording_at(request.key, request.position)?;
            }
            self.update_video_recording()?;
        }
        Ok(())
    }

    fn show_track_add_menu(&self, request: TrackAddMenuRequest) -> Result<(), String> {
        let key = request.key;
        let session = &self.ivars().session;
        let import_targets = request.import_targets;
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
        let entries = shrimply_timeline_skia::track_add_menu(key.kind);
        for (index, entry) in entries.iter().enumerate() {
            let TrackAddMenuEntry::Action(action) = entry else {
                menu.addItem(&NSMenuItem::separatorItem(self.mtm()));
                continue;
            };
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(self.mtm()),
                    &NSString::from_str(shrimply_i18n::text(action.label(key.kind)).as_ref()),
                    Some(sel!(chooseTrackAdd:)),
                    ns_string!(""),
                )
            };
            item.setTag(index.try_into().expect("track menu index fits NSInteger"));
            unsafe {
                item.setTarget(Some(self));
            }
            item.setImage(Some(&shrimply_components_appkit::bundled_symbol(
                action.icon(),
                action.label(key.kind),
            )));
            menu.addItem(&item);
        }
        self.ivars().menu_choice.set(None);
        // Native menus run a nested event loop; hold no scene or project borrow here.
        super::super::menu_sheet::present(self, &menu, request.point, request.touch);
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
                request.position,
                self.mtm(),
            );
        }
        let preferences = shrimply_editor_state::preferences::snapshot(&session.preferences);
        shrimply_timeline_skia::activate_track_add_checked_at(
            &session.project,
            &session.player_state,
            &session.selection_state,
            key,
            action,
            TrackAddSettings {
                default_visual_duration: preferences.default_visual_duration,
                default_text_font_family: &preferences.default_text_font_family,
            },
            request.position,
        )?;
        Ok(())
    }
}
