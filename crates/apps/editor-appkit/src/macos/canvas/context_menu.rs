use super::*;
use objc2::sel;
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSMenu, NSMenuItem, NSPasteboard, NSPasteboardTypeString,
    NSSlider, NSTextField, NSWorkspace,
};
use objc2_foundation::{NSPoint, NSString, NSURL, ns_string};
use shrimply_timeline_core::{ContextMenuEntry, ContextMenuRequest, TIMELINE_CLIPBOARD_MARKER};

const MENU_CONTROL_WIDTH: f64 = 232.0;
const MENU_CONTROL_HEIGHT: f64 =
    MENU_SLIDER_HEIGHT + MENU_LABEL_HEIGHT + MENU_CONTROL_PADDING * 3.0;
const MENU_CONTROL_PADDING: f64 = 12.0;
const MENU_LABEL_HEIGHT: f64 = 18.0;
const MENU_SLIDER_HEIGHT: f64 = 24.0;

impl CanvasView {
    pub(super) fn open_context_menu(&self, event: &NSEvent) {
        if let Some(window) = self.window() {
            window.makeFirstResponder(Some(self));
        }
        if matches!(&*self.ivars().content.borrow(), Content::Preview(_)) {
            self.open_preview_context_menu(event);
            return;
        }
        let point = self.point(event);
        let definition = {
            let mut content = self.ivars().content.borrow_mut();
            let Content::Timeline(scene) = &mut *content else {
                return;
            };
            scene.prepare_context_menu(point)
        };
        if definition.sections.is_empty() {
            return;
        }
        self.ivars().menu_choice.set(None);
        self.ivars().context_error.replace(None);
        self.ivars().context_controls.borrow_mut().clear();
        let menu = NSMenu::initWithTitle(NSMenu::alloc(self.mtm()), ns_string!("Timeline"));
        menu.setAutoenablesItems(false);
        let mut actions = Vec::new();
        for section in &definition.sections {
            if section.is_empty() {
                continue;
            }
            if menu.numberOfItems() > 0 {
                menu.addItem(&NSMenuItem::separatorItem(self.mtm()));
            }
            for entry in section {
                match *entry {
                    ContextMenuEntry::Action(action) => {
                        let item = unsafe {
                            NSMenuItem::initWithTitle_action_keyEquivalent(
                                NSMenuItem::alloc(self.mtm()),
                                &NSString::from_str(action.label()),
                                Some(sel!(chooseCanvasContext:)),
                                ns_string!(""),
                            )
                        };
                        item.setTag(
                            actions
                                .len()
                                .try_into()
                                .expect("menu action index fits NSInteger"),
                        );
                        item.setEnabled(action.enabled);
                        unsafe {
                            item.setTarget(Some(self));
                        }
                        actions.push(action.action);
                        menu.addItem(&item);
                    }
                    ContextMenuEntry::Control(control) => {
                        let container = NSView::initWithFrame(
                            NSView::alloc(self.mtm()),
                            NSRect::new(
                                NSPoint::ZERO,
                                NSSize::new(MENU_CONTROL_WIDTH, MENU_CONTROL_HEIGHT),
                            ),
                        );
                        let label = NSTextField::labelWithString(
                            &NSString::from_str(&format!(
                                "{}{}",
                                control.label(),
                                if control.mixed() { " — Mixed" } else { "" }
                            )),
                            self.mtm(),
                        );
                        label.setFrame(NSRect::new(
                            NSPoint::new(
                                MENU_CONTROL_PADDING,
                                MENU_CONTROL_HEIGHT - MENU_CONTROL_PADDING - MENU_LABEL_HEIGHT,
                            ),
                            NSSize::new(
                                MENU_CONTROL_WIDTH - MENU_CONTROL_PADDING * 2.0,
                                MENU_LABEL_HEIGHT,
                            ),
                        ));
                        container.addSubview(&label);
                        let slider = unsafe {
                            NSSlider::sliderWithTarget_action(
                                Some(self),
                                Some(sel!(changeTimelineContextControl:)),
                                self.mtm(),
                            )
                        };
                        slider.setMinValue(control.minimum());
                        slider.setMaxValue(control.maximum());
                        slider.setDoubleValue(control.value());
                        slider.setAltIncrementValue(control.step());
                        slider.setContinuous(false);
                        slider.setTag(
                            self.ivars()
                                .context_controls
                                .borrow()
                                .len()
                                .try_into()
                                .expect("menu control index fits NSInteger"),
                        );
                        self.ivars().context_controls.borrow_mut().push(control);
                        slider.setFrame(NSRect::new(
                            NSPoint::new(MENU_CONTROL_PADDING, MENU_CONTROL_PADDING),
                            NSSize::new(
                                MENU_CONTROL_WIDTH - MENU_CONTROL_PADDING * 2.0,
                                MENU_SLIDER_HEIGHT,
                            ),
                        ));
                        container.addSubview(&slider);
                        let item = unsafe {
                            NSMenuItem::initWithTitle_action_keyEquivalent(
                                NSMenuItem::alloc(self.mtm()),
                                &NSString::from_str(control.label()),
                                None,
                                ns_string!(""),
                            )
                        };
                        item.setView(Some(&container));
                        menu.addItem(&item);
                    }
                }
            }
        }
        // Native menus run a nested event loop; no scene/project borrow crosses it.
        menu.popUpMenuPositioningItem_atLocation_inView(
            None,
            NSPoint::new(point.x.into(), point.y.into()),
            Some(self),
        );
        let context_error = self.ivars().context_error.borrow_mut().take();
        let result = if let Some(error) = context_error {
            Err(error)
        } else if let Some(index) = self.ivars().menu_choice.take() {
            let action = actions[index];
            let request = {
                let mut content = self.ivars().content.borrow_mut();
                let Content::Timeline(scene) = &mut *content else {
                    return;
                };
                scene.activate_context_menu_action(action)
            };
            request.and_then(|request| {
                if let Some(request) = request {
                    self.handle_context_request(request)
                } else {
                    Ok(())
                }
            })
        } else {
            Ok(())
        };
        if let Err(error) = result {
            self.show_error(&error);
        }
        self.ivars().context_controls.borrow_mut().clear();
        self.update_tracking();
    }

    pub(super) fn change_context_control(&self, slider: &NSSlider) {
        let control = self
            .ivars()
            .context_controls
            .borrow()
            .get(usize::try_from(slider.tag()).expect("menu control tag is nonnegative"))
            .copied();
        let Some(control) = control else { return };
        let result = {
            let mut content = self.ivars().content.borrow_mut();
            let Content::Timeline(scene) = &mut *content else {
                return;
            };
            scene.set_context_menu_control(control, slider.doubleValue())
        };
        if let Err(error) = result {
            self.ivars().context_error.replace(Some(error));
        }
    }

    pub(super) fn handle_context_request(&self, request: ContextMenuRequest) -> Result<(), String> {
        match request {
            ContextMenuRequest::SetTimelineClipboardMarker => {
                let clipboard = NSPasteboard::generalPasteboard();
                clipboard.clearContents();
                if !clipboard.setString_forType(
                    &NSString::from_str(TIMELINE_CLIPBOARD_MARKER),
                    unsafe { NSPasteboardTypeString },
                ) {
                    return Err("Could not write the timeline clipboard".into());
                }
                Ok(())
            }
            ContextMenuRequest::PasteFromClipboard => {
                let clipboard = NSPasteboard::generalPasteboard();
                let urls = super::super::media::file_urls(&clipboard);
                if !urls.is_empty() {
                    let placement = shrimply_timeline_core::import_queue::Placement {
                        start: shrimply_state::player_state::current_time(
                            &self.ivars().session.player_state,
                        ),
                        target: shrimply_timeline_core::items::NewItemTarget::Automatic,
                        collision: shrimply_timeline_core::DragCollisionMode::NewTrack,
                    };
                    return self.ivars().imports.borrow_mut().enqueue(
                        urls,
                        &self.ivars().session,
                        super::super::media::Destination::Timeline(placement),
                    );
                }
                if clipboard.stringForType(unsafe { NSPasteboardTypeString }).is_some_and(|text| text.to_string() == TIMELINE_CLIPBOARD_MARKER) {
                    let mut content = self.ivars().content.borrow_mut();
                    if let Content::Timeline(scene) = &mut *content {
                        return scene.paste_context_clipboard();
                    }
                }
                Err("The clipboard does not contain copied timeline items or media files".into())
            }
            ContextMenuRequest::ShowInFolder => {
                let path = {
                    let content = self.ivars().content.borrow();
                    if let Content::Timeline(scene) = &*content {
                        scene.context_file_path()
                    } else {
                        None
                    }
                }.ok_or("Selected item has no source file")?;
                let url = NSURL::from_file_path(&path).ok_or("Could not resolve the source file URL")?;
                NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_slice(&[&*url]));
                Ok(())
            }
            ContextMenuRequest::DeleteFoldedTrack { clip_count } | ContextMenuRequest::DeleteTracks { clip_count } => {
                let deletion = match &*self.ivars().content.borrow() {
                    Content::Timeline(scene) => Some(scene.track_deletion()),
                    _ => None,
                };
                let alert = NSAlert::new(self.mtm());
                alert.setMessageText(ns_string!("Delete Track?"));
                alert.setInformativeText(&NSString::from_str(&format!("This track contains {clip_count} clips. Deleting it removes those clips.")));
                alert.addButtonWithTitle(ns_string!("Delete"));
                alert.addButtonWithTitle(ns_string!("Cancel"));
                if alert.runModal() != NSAlertFirstButtonReturn {
                    return Ok(());
                }
                let mut content = self.ivars().content.borrow_mut();
                if let Content::Timeline(scene) = &mut *content {
                    if matches!(request, ContextMenuRequest::DeleteTracks { .. }) {
                        scene.confirm_delete_selected_tracks(deletion.ok_or("Timeline was closed while confirming deletion")?)
                    } else {
                        scene.confirm_delete_context_track()
                    }
                } else {
                    Ok(())
                }
            }
            ContextMenuRequest::ExportAudio => self.export_selected_audio(),
            ContextMenuRequest::CopyFrame(selection) => self.capture_selected_frame(selection, false),
            ContextMenuRequest::SaveFrame(selection) => self.capture_selected_frame(selection, true),
            ContextMenuRequest::Transcribe => Err("Transcription requires its model and options dialog, which is not yet connected to the AppKit editor".into()),
            ContextMenuRequest::RemoveSilences => Err("Silence removal requires its analysis and options dialog, which is not yet connected to the AppKit editor".into()),
            ContextMenuRequest::GenerateSpeech => Err("Speech generation requires its voice and model dialog, which is not yet connected to the AppKit editor".into()),
        }
    }
}
