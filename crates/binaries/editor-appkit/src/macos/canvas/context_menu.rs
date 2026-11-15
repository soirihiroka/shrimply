use super::*;
use block2::RcBlock;
use objc2::sel;
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSButton, NSControlStateValueOff, NSControlStateValueOn,
    NSFont, NSMenu, NSMenuItem, NSModalResponseOK, NSModalResponseStop, NSOpenPanel, NSPasteboard,
    NSPasteboardTypeString, NSPopUpButton, NSProgressIndicator, NSProgressIndicatorStyle, NSSlider,
    NSTextField, NSTextView, NSWorkspace,
};
use objc2_foundation::{NSNumber, NSPoint, NSString, NSURL, ns_string};
use shrimply_timeline_skia::{ContextMenuEntry, ContextMenuRequest, TIMELINE_CLIPBOARD_MARKER};
use std::sync::mpsc::{self, TryRecvError};

const MENU_CONTROL_WIDTH: f64 = 232.0;
const MENU_CONTROL_HEIGHT: f64 =
    MENU_SLIDER_HEIGHT + MENU_LABEL_HEIGHT + MENU_CONTROL_PADDING * 3.0;
const MENU_CONTROL_PADDING: f64 = 12.0;
const MENU_LABEL_HEIGHT: f64 = 18.0;
const MENU_SLIDER_HEIGHT: f64 = 24.0;
const TTS_TABLE_WIDTH: f64 = 520.0;
const TTS_TABLE_HEIGHT: f64 = 240.0;
const TTS_TABLE_FONT_SIZE: f64 = 12.0;

pub(super) struct TranscriptionProbe {
    receiver: mpsc::Receiver<Result<Vec<String>, String>>,
    alert: Retained<NSAlert>,
    server_url: String,
}

pub(super) struct CaptionSpeechProbe {
    receiver: mpsc::Receiver<Result<Vec<shrimply_tts::TtsModel>, String>>,
    alert: Retained<NSAlert>,
    server_url: String,
    plan: shrimply_timeline_skia::caption_speech::Plan,
}

impl CanvasView {
    pub(super) fn open_context_menu(&self, event: &NSEvent) {
        if let Some(window) = self.window() {
            window.makeFirstResponder(Some(self));
        }
        if matches!(&*self.ivars().content.borrow(), Content::Preview(_)) {
            self.open_preview_context_menu(event);
            return;
        }
        self.open_timeline_context_menu(self.point(event), false);
    }

    pub(super) fn open_timeline_context_menu(&self, point: glam::Vec2, touch: bool) {
        let definition = {
            let mut content = self.ivars().content.borrow_mut();
            let Content::Timeline(scene) = &mut *content else {
                return;
            };
            scene.prepare_context_menu_for_input(point, touch)
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
                                &NSString::from_str(shrimply_i18n::text(action.label()).as_ref()),
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
                        if let Some(checked) = action.checked {
                            unsafe {
                                item.setRepresentedObject(Some(&NSNumber::new_bool(checked)));
                            }
                            item.setState(if checked {
                                NSControlStateValueOn
                            } else {
                                NSControlStateValueOff
                            });
                        }
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
                                shrimply_i18n::text(control.label()),
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
                                &NSString::from_str(shrimply_i18n::text(control.label()).as_ref()),
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
        super::super::menu_sheet::present(self, &menu, point, touch);
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
                if !clipboard
                    .setString_forType(&NSString::from_str(TIMELINE_CLIPBOARD_MARKER), unsafe {
                        NSPasteboardTypeString
                    })
                {
                    return Err("Could not write the timeline clipboard".into());
                }
                Ok(())
            }
            ContextMenuRequest::PasteFromClipboard => {
                let clipboard = NSPasteboard::generalPasteboard();
                let urls = super::super::media::stage_clipboard_file_urls(
                    super::super::media::file_urls(&clipboard),
                )?;
                if !urls.is_empty() {
                    return self.perform_external_file_urls(urls, None);
                }
                if let Some(path) = super::super::media::clipboard_image_path(&clipboard)? {
                    return self.perform_external_drop(
                        shrimply_timeline_skia::external_content::ExternalDrop::Files(vec![path]),
                        None,
                    );
                }
                if let Some(text) = clipboard.stringForType(unsafe { NSPasteboardTypeString }) {
                    let text = text.to_string();
                    if text == TIMELINE_CLIPBOARD_MARKER {
                        let mut content = self.ivars().content.borrow_mut();
                        if let Content::Timeline(scene) = &mut *content {
                            return scene.paste_context_clipboard();
                        }
                    } else {
                        let content =
                            match shrimply_timeline_skia::external_content::classify_external_text(
                                text,
                            ) {
                                shrimply_timeline_skia::external_content::ExternalText::Text(
                                    text,
                                ) => {
                                    shrimply_timeline_skia::external_content::ExternalDrop::Text(
                                        text,
                                    )
                                }
                                shrimply_timeline_skia::external_content::ExternalText::ImageUrl(
                                    url,
                                ) => {
                                    shrimply_timeline_skia::external_content::ExternalDrop::ImageUrl(
                                        url,
                                    )
                                }
                            };
                        return self.perform_external_drop(content, None);
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
                }
                .ok_or("Selected item has no source file")?;
                let url =
                    NSURL::from_file_path(&path).ok_or("Could not resolve the source file URL")?;
                NSWorkspace::sharedWorkspace()
                    .activateFileViewerSelectingURLs(&NSArray::from_slice(&[&*url]));
                Ok(())
            }
            ContextMenuRequest::DeleteFoldedTrack { clip_count }
            | ContextMenuRequest::DeleteTracks { clip_count } => {
                let deletion = match &*self.ivars().content.borrow() {
                    Content::Timeline(scene) => Some(scene.track_deletion()),
                    _ => None,
                };
                let alert = NSAlert::new(self.mtm());
                alert.setMessageText(ns_string!("Delete Track?"));
                alert.setInformativeText(&NSString::from_str(&format!(
                    "This track contains {clip_count} clips. Deleting it removes those clips."
                )));
                alert.addButtonWithTitle(ns_string!("Delete"));
                alert.addButtonWithTitle(ns_string!("Cancel"));
                if alert.runModal() != NSAlertFirstButtonReturn {
                    return Ok(());
                }
                let mut content = self.ivars().content.borrow_mut();
                if let Content::Timeline(scene) = &mut *content {
                    if matches!(request, ContextMenuRequest::DeleteTracks { .. }) {
                        scene.confirm_delete_selected_tracks(
                            deletion.ok_or("Timeline was closed while confirming deletion")?,
                        )
                    } else {
                        scene.confirm_delete_context_track()
                    }
                } else {
                    Ok(())
                }
            }
            ContextMenuRequest::ExportAudio => self.export_selected_audio(),
            ContextMenuRequest::CopyFrame(selection) => {
                self.capture_selected_frame(selection, false)
            }
            ContextMenuRequest::SaveFrame(selection) => {
                self.capture_selected_frame(selection, true)
            }
            ContextMenuRequest::Transcribe => self.transcribe(),
            ContextMenuRequest::RemoveSilences => self.remove_silences(),
            ContextMenuRequest::GenerateSpeech => self.generate_speech(),
        }
    }

    fn remove_silences(&self) -> Result<(), String> {
        const WIDTH: f64 = 360.0;
        const ROW_HEIGHT: f64 = 30.0;
        const LABEL_WIDTH: f64 = 220.0;
        const FIELD_WIDTH: f64 = WIDTH - LABEL_WIDTH;
        const FIELD_HEIGHT: f64 = 24.0;
        const LABEL_HEIGHT: f64 = 22.0;
        const LABEL_Y_OFFSET: f64 = 4.0;
        let defaults = shrimply_timeline_skia::silence::Config::default();
        let values = [
            ("Silence threshold (dB)", defaults.threshold_db),
            (
                "Minimum silence (seconds)",
                defaults.min_silence.as_secs_f64(),
            ),
            (
                "Gap tolerance (seconds)",
                defaults.gap_tolerance.as_secs_f64(),
            ),
            ("Padding (seconds)", defaults.padding.as_secs_f64()),
            (
                "Minimum chunk (seconds)",
                defaults.delete_chunks.as_secs_f64(),
            ),
        ];
        let row_count = values.len();
        let accessory = NSView::initWithFrame(
            NSView::alloc(self.mtm()),
            NSRect::new(
                NSPoint::ZERO,
                NSSize::new(WIDTH, ROW_HEIGHT * row_count as f64),
            ),
        );
        let mut fields = Vec::with_capacity(values.len());
        for (index, (title, value)) in values.iter().copied().enumerate() {
            let y = ROW_HEIGHT * (row_count - index - 1) as f64;
            let label = NSTextField::labelWithString(&NSString::from_str(title), self.mtm());
            label.setFrame(NSRect::new(
                NSPoint::new(0.0, y + LABEL_Y_OFFSET),
                NSSize::new(LABEL_WIDTH, LABEL_HEIGHT),
            ));
            accessory.addSubview(&label);
            let field = NSTextField::initWithFrame(
                NSTextField::alloc(self.mtm()),
                NSRect::new(
                    NSPoint::new(LABEL_WIDTH, y),
                    NSSize::new(FIELD_WIDTH, FIELD_HEIGHT),
                ),
            );
            field.setDoubleValue(value);
            accessory.addSubview(&field);
            fields.push(field);
        }

        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(ns_string!("Remove Silences"));
        alert.setInformativeText(ns_string!(
            "Remove quiet ranges from the selected audio and ripple the timeline."
        ));
        alert.addButtonWithTitle(ns_string!("Remove Silences"));
        alert.addButtonWithTitle(ns_string!("Cancel"));
        alert.setAccessoryView(Some(&accessory));
        if alert.runModal() != NSAlertFirstButtonReturn {
            return Ok(());
        }

        let seconds = |index: usize, maximum: f64| -> Result<_, String> {
            let value = fields[index].doubleValue();
            if !value.is_finite() || !(0.0..=maximum).contains(&value) {
                return Err(format!(
                    "{} must be between 0 and {maximum} seconds",
                    values[index].0
                ));
            }
            Ok(shrimply_project_document::project::Time::from_seconds_f64(
                value,
            ))
        };
        let threshold = fields[0].doubleValue();
        if !threshold.is_finite() || !(-90.0..=0.0).contains(&threshold) {
            return Err("Silence threshold must be between -90 and 0 dB".into());
        }
        let removed = {
            let mut content = self.ivars().content.borrow_mut();
            let Content::Timeline(scene) = &mut *content else {
                return Err("timeline closed while removing silences".into());
            };
            scene.remove_silences(shrimply_timeline_skia::silence::Config {
                threshold_db: threshold,
                min_silence: seconds(1, 10.0)?,
                gap_tolerance: seconds(2, 5.0)?,
                padding: seconds(3, 2.0)?,
                delete_chunks: seconds(4, 10.0)?,
            })?
        };
        if removed == 0 {
            let alert = NSAlert::new(self.mtm());
            alert.setMessageText(ns_string!("No Silences Found"));
            alert.setInformativeText(ns_string!(
                "No selected-audio gaps matched the configured threshold and duration."
            ));
            alert.addButtonWithTitle(ns_string!("OK"));
            alert.runModal();
        }
        Ok(())
    }

    fn generate_speech(&self) -> Result<(), String> {
        if self.ivars().caption_speech_probe.borrow().is_some()
            || self.ivars().caption_speech_alert.borrow().is_some()
        {
            return Err("A speech-generation workflow is already open".into());
        }
        let plan = match &*self.ivars().content.borrow() {
            Content::Timeline(scene) => scene.caption_speech_plan()?,
            _ => return Err("Timeline closed while preparing speech generation".into()),
        };
        let preferences =
            shrimply_editor_state::preferences::snapshot(&self.ivars().session.preferences);
        let server_url = preferences.compute_server_url;
        if server_url.trim().is_empty() {
            return Err("Set a compute server in Settings before generating speech".into());
        }
        let (sender, receiver) = mpsc::channel();
        let probe_url = server_url.clone();
        std::thread::Builder::new()
            .name("caption-speech-models".into())
            .spawn(move || {
                let _ = sender.send(shrimply_timeline_skia::caption_speech::models(&probe_url));
            })
            .map_err(|error| format!("Could not start speech model lookup: {error}"))?;
        let spinner = NSProgressIndicator::initWithFrame(
            NSProgressIndicator::alloc(self.mtm()),
            NSRect::new(NSPoint::ZERO, NSSize::new(360.0, 26.0)),
        );
        spinner.setStyle(NSProgressIndicatorStyle::Spinning);
        unsafe { spinner.startAnimation(None) };
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(ns_string!("Generate Speech"));
        alert.setInformativeText(ns_string!("Loading text-to-speech models…"));
        alert.setAccessoryView(Some(&spinner));
        self.ivars()
            .caption_speech_probe
            .replace(Some(CaptionSpeechProbe {
                receiver,
                alert: alert.clone(),
                server_url,
                plan,
            }));
        alert.beginSheetModalForWindow_completionHandler(
            &self.window().expect("canvas must be attached"),
            None,
        );
        Ok(())
    }

    pub(super) fn poll_caption_speech_probe(&self) -> Result<(), String> {
        let result = {
            let probe = self.ivars().caption_speech_probe.borrow();
            let Some(probe) = probe.as_ref() else {
                return Ok(());
            };
            match probe.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some(Err("Speech model lookup stopped unexpectedly".into()))
                }
            }
        };
        let Some(result) = result else {
            return Ok(());
        };
        let probe = self
            .ivars()
            .caption_speech_probe
            .borrow_mut()
            .take()
            .expect("completed speech model probe exists");
        close_alert(&probe.alert);
        self.present_caption_speech_dialog(probe.plan, probe.server_url, result?)
    }

    fn present_caption_speech_dialog(
        &self,
        plan: shrimply_timeline_skia::caption_speech::Plan,
        server_url: String,
        models: Vec<shrimply_tts::TtsModel>,
    ) -> Result<(), String> {
        if models.is_empty() {
            return Err("The server has not provided a text-to-speech model".into());
        }
        let remembered =
            shrimply_editor_state::preferences::snapshot(&self.ivars().session.preferences)
                .last_tts_model;
        let model_menu = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(self.mtm()),
            NSRect::new(NSPoint::ZERO, NSSize::new(360.0, 26.0)),
            false,
        );
        for model in &models {
            model_menu.addItemWithTitle(&NSString::from_str(&model.label));
        }
        if let Some(index) = models.iter().position(|model| model.id == remembered) {
            model_menu.selectItemAtIndex(index as isize);
        }
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(ns_string!("Generate Speech"));
        alert.setInformativeText(&NSString::from_str(&format!(
            "{} caption chunk{} will be generated.{}",
            plan.chunks(),
            if plan.chunks() == 1 { "" } else { "s" },
            if plan.skipped() == 0 {
                String::new()
            } else {
                format!(
                    " {} empty or invalid captions will be skipped.",
                    plan.skipped()
                )
            }
        )));
        alert.setAccessoryView(Some(&model_menu));
        alert.addButtonWithTitle(ns_string!("Continue"));
        alert.addButtonWithTitle(ns_string!("Cancel"));
        if alert.runModal() != NSAlertFirstButtonReturn {
            return Ok(());
        }
        let index = usize::try_from(model_menu.indexOfSelectedItem())
            .map_err(|_| "No text-to-speech model is selected")?;
        let model = models
            .get(index)
            .cloned()
            .ok_or("No text-to-speech model is selected")?;
        let Some(settings) = self.configure_caption_speech_model(&model)? else {
            return Ok(());
        };
        let handle = match &mut *self.ivars().content.borrow_mut() {
            Content::Timeline(scene) => scene.start_caption_speech(
                plan,
                shrimply_timeline_skia::caption_speech::Options {
                    server_url,
                    model: model.clone(),
                    settings,
                },
            )?,
            _ => return Err("Timeline closed while starting speech generation".into()),
        };
        shrimply_editor_state::preferences::set_last_tts_model(
            &self.ivars().session.preferences,
            &model.id,
        );
        let progress = NSProgressIndicator::initWithFrame(
            NSProgressIndicator::alloc(self.mtm()),
            NSRect::new(NSPoint::ZERO, NSSize::new(360.0, 26.0)),
        );
        progress.setStyle(NSProgressIndicatorStyle::Spinning);
        unsafe { progress.startAnimation(None) };
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(ns_string!("Generating Speech"));
        alert.setInformativeText(&NSString::from_str(&format!(
            "{} · Sending request…",
            model.label
        )));
        alert.setAccessoryView(Some(&progress));
        alert.addButtonWithTitle(ns_string!("Cancel"));
        let completion = RcBlock::new(move |response| {
            if response == NSAlertFirstButtonReturn {
                handle.cancel();
            }
        });
        self.ivars()
            .caption_speech_alert
            .replace(Some(alert.clone()));
        alert.beginSheetModalForWindow_completionHandler(
            &self.window().expect("canvas must be attached"),
            Some(&completion),
        );
        Ok(())
    }

    fn configure_caption_speech_model(
        &self,
        model: &shrimply_tts::TtsModel,
    ) -> Result<Option<shrimply_tts::TtsSettings>, String> {
        let mut settings = shrimply_tts::TtsSettings::default();
        shrimply_tts::sync_settings(&mut settings, model);
        for input in &model.inputs {
            if !shrimply_tts::is_visible(input, &settings.inputs) {
                continue;
            }
            use shrimply_tts::{InputDefinition as I, InputPurpose, TtsValue};
            match input {
                I::Text {
                    key,
                    label,
                    purpose: Some(InputPurpose::Text),
                    ..
                }
                | I::Number {
                    key,
                    label,
                    purpose: Some(InputPurpose::Duration | InputPurpose::SpeedFactor),
                    ..
                } => {
                    let _ = (key, label);
                }
                I::Select { options, .. }
                    if options.iter().any(|option| {
                        matches!(
                            option.purpose,
                            Some(InputPurpose::Duration | InputPurpose::SpeedFactor)
                        )
                    }) => {}
                I::Text {
                    key,
                    label,
                    default,
                    ..
                } => {
                    let field = NSTextField::initWithFrame(
                        NSTextField::alloc(self.mtm()),
                        NSRect::new(NSPoint::ZERO, NSSize::new(360.0, 26.0)),
                    );
                    field.setStringValue(&NSString::from_str(default));
                    if !self.confirm_tts_control(label, &field) {
                        return Ok(None);
                    }
                    settings.inputs.insert(
                        key.clone(),
                        TtsValue::Text {
                            value: field.stringValue().to_string(),
                        },
                    );
                }
                I::Select {
                    key,
                    label,
                    options,
                    default,
                    ..
                } => {
                    let menu = NSPopUpButton::initWithFrame_pullsDown(
                        NSPopUpButton::alloc(self.mtm()),
                        NSRect::new(NSPoint::ZERO, NSSize::new(360.0, 26.0)),
                        false,
                    );
                    for option in options {
                        menu.addItemWithTitle(&NSString::from_str(&option.label));
                    }
                    if let Some(index) = options.iter().position(|option| option.value == *default)
                    {
                        menu.selectItemAtIndex(index as isize);
                    }
                    if !self.confirm_tts_control(label, &menu) {
                        return Ok(None);
                    }
                    let index = usize::try_from(menu.indexOfSelectedItem())
                        .map_err(|_| format!("{label} has no selection"))?;
                    let value = options
                        .get(index)
                        .ok_or_else(|| format!("{label} has no selection"))?
                        .value
                        .clone();
                    settings
                        .inputs
                        .insert(key.clone(), TtsValue::Select { value });
                }
                I::Audio {
                    key,
                    label,
                    required,
                    ..
                } => {
                    let panel = NSOpenPanel::openPanel(self.mtm());
                    panel.setCanChooseDirectories(false);
                    panel.setAllowsMultipleSelection(false);
                    panel.setTitle(Some(&NSString::from_str(label)));
                    if panel.runModal() != NSModalResponseOK {
                        if *required {
                            return Ok(None);
                        }
                        continue;
                    }
                    let path = panel
                        .URL()
                        .ok_or_else(|| format!("{label} selection has no URL"))?
                        .to_file_path()
                        .ok_or_else(|| format!("{label} must be a local file"))?;
                    settings.inputs.insert(
                        key.clone(),
                        TtsValue::Audio {
                            value: shrimply_asset::Asset::new(path),
                        },
                    );
                }
                I::Toggle {
                    key,
                    label,
                    default,
                    ..
                } => {
                    let checkbox = unsafe {
                        NSButton::checkboxWithTitle_target_action(
                            &NSString::from_str(label),
                            None,
                            None,
                            self.mtm(),
                        )
                    };
                    checkbox.setState(if *default {
                        NSControlStateValueOn
                    } else {
                        NSControlStateValueOff
                    });
                    if !self.confirm_tts_control(label, &checkbox) {
                        return Ok(None);
                    }
                    settings.inputs.insert(
                        key.clone(),
                        TtsValue::Toggle {
                            value: checkbox.state() == NSControlStateValueOn,
                        },
                    );
                }
                I::Number {
                    key,
                    label,
                    default,
                    minimum,
                    maximum,
                    ..
                } => {
                    let field = NSTextField::initWithFrame(
                        NSTextField::alloc(self.mtm()),
                        NSRect::new(NSPoint::ZERO, NSSize::new(360.0, 26.0)),
                    );
                    field.setDoubleValue(shrimply_math_core::fraction_as_f64(*default));
                    if !self.confirm_tts_control(label, &field) {
                        return Ok(None);
                    }
                    let value = field.doubleValue();
                    if !value.is_finite()
                        || value < shrimply_math_core::fraction_as_f64(*minimum)
                        || value > shrimply_math_core::fraction_as_f64(*maximum)
                    {
                        return Err(format!("{label} is outside its allowed range"));
                    }
                    settings.inputs.insert(
                        key.clone(),
                        TtsValue::Number {
                            value: shrimply_math_core::fraction_from_f64(value),
                        },
                    );
                }
                I::Table {
                    key,
                    label,
                    columns,
                    ..
                } => {
                    if columns.is_empty() {
                        return Err(format!("{label} has no columns"));
                    }
                    let scroll = NSTextView::scrollableTextView(self.mtm());
                    scroll.setFrame(NSRect::new(
                        NSPoint::ZERO,
                        NSSize::new(TTS_TABLE_WIDTH, TTS_TABLE_HEIGHT),
                    ));
                    let editor = scroll
                        .documentView()
                        .and_then(|view| view.downcast::<NSTextView>().ok())
                        .expect("AppKit's scrollable text view must contain an NSTextView");
                    editor.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(
                        TTS_TABLE_FONT_SIZE,
                        unsafe { objc2_app_kit::NSFontWeightRegular },
                    )));
                    let current = settings
                        .inputs
                        .get(key)
                        .and_then(|value| match value {
                            TtsValue::Table { rows } => Some(rows),
                            _ => None,
                        })
                        .into_iter()
                        .flatten()
                        .map(|row| {
                            columns
                                .iter()
                                .map(|column| row.get(&column.key).map_or("", String::as_str))
                                .collect::<Vec<_>>()
                                .join("\t")
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    editor.setString(&NSString::from_str(&current));
                    let column_labels = columns
                        .iter()
                        .map(|column| column.label.as_str())
                        .collect::<Vec<_>>()
                        .join(" · ");
                    if !self.confirm_tts_control(
                        &format!("{label} — one tab-separated row per line: {column_labels}"),
                        &scroll,
                    ) {
                        return Ok(None);
                    }
                    let mut rows = Vec::new();
                    for (row_index, line) in editor.string().to_string().lines().enumerate() {
                        if line.trim().is_empty() {
                            continue;
                        }
                        let values = line.split('\t').collect::<Vec<_>>();
                        if values.len() != columns.len() {
                            return Err(format!(
                                "{label} row {} needs {} tab-separated values",
                                row_index + 1,
                                columns.len()
                            ));
                        }
                        let mut row = std::collections::BTreeMap::new();
                        for (column, value) in columns.iter().zip(values) {
                            if column.required && value.trim().is_empty() {
                                return Err(format!(
                                    "{label} row {} requires {}",
                                    row_index + 1,
                                    column.label
                                ));
                            }
                            if value.chars().count() > column.max_length {
                                return Err(format!(
                                    "{label} row {} column {} exceeds {} characters",
                                    row_index + 1,
                                    column.label,
                                    column.max_length
                                ));
                            }
                            row.insert(column.key.clone(), value.to_string());
                        }
                        rows.push(row);
                    }
                    settings
                        .inputs
                        .insert(key.clone(), TtsValue::Table { rows });
                }
            }
        }
        Ok(Some(settings))
    }

    fn confirm_tts_control(&self, label: &str, control: &NSView) -> bool {
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(&NSString::from_str(label));
        alert.setAccessoryView(Some(control));
        alert.addButtonWithTitle(ns_string!("Continue"));
        alert.addButtonWithTitle(ns_string!("Cancel"));
        alert.runModal() == NSAlertFirstButtonReturn
    }

    pub(super) fn handle_caption_speech_update(
        &self,
        update: shrimply_timeline_skia::caption_speech::Update,
    ) -> Result<(), String> {
        match update {
            shrimply_timeline_skia::caption_speech::Update::Progress {
                current,
                total,
                message,
                preview,
            } => {
                if let Some(alert) = self.ivars().caption_speech_alert.borrow().as_ref() {
                    alert.setInformativeText(&NSString::from_str(&format!(
                        "Chunk {current}/{total} · {message}\n{preview}"
                    )));
                }
                return Ok(());
            }
            shrimply_timeline_skia::caption_speech::Update::Failed(error) => {
                self.close_caption_speech_alert();
                return Err(error);
            }
            shrimply_timeline_skia::caption_speech::Update::Finished(summary) => {
                self.close_caption_speech_alert();
                if summary.failed > 0
                    || summary.skipped > 0
                    || summary.unattempted > 0
                    || summary.cancelled
                {
                    return Err(summary.to_string());
                }
            }
        }
        Ok(())
    }

    fn close_caption_speech_alert(&self) {
        let Some(alert) = self.ivars().caption_speech_alert.borrow_mut().take() else {
            return;
        };
        close_alert(&alert);
    }

    fn transcribe(&self) -> Result<(), String> {
        if self.ivars().transcription_probe.borrow().is_some()
            || self.ivars().transcription_alert.borrow().is_some()
        {
            return Err("A transcription workflow is already open".into());
        }
        let server_url =
            shrimply_editor_state::preferences::snapshot(&self.ivars().session.preferences)
                .compute_server_url;
        if server_url.trim().is_empty() {
            return Err("Set a compute server in Settings before transcribing audio".into());
        }
        let (sender, receiver) = mpsc::channel();
        let probe_url = server_url.clone();
        std::thread::Builder::new()
            .name("transcription-models".into())
            .spawn(move || {
                let _ = sender.send(shrimply_timeline_skia::transcription::models(&probe_url));
            })
            .map_err(|error| format!("Could not start transcription model lookup: {error}"))?;
        let spinner = NSProgressIndicator::initWithFrame(
            NSProgressIndicator::alloc(self.mtm()),
            NSRect::new(NSPoint::ZERO, NSSize::new(360.0, 26.0)),
        );
        spinner.setStyle(NSProgressIndicatorStyle::Spinning);
        unsafe { spinner.startAnimation(None) };
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(ns_string!("Transcribe"));
        alert.setInformativeText(ns_string!("Loading speech-to-text models…"));
        alert.setAccessoryView(Some(&spinner));
        self.ivars()
            .transcription_probe
            .replace(Some(TranscriptionProbe {
                receiver,
                alert: alert.clone(),
                server_url,
            }));
        alert.beginSheetModalForWindow_completionHandler(
            &self.window().expect("canvas must be attached"),
            None,
        );
        Ok(())
    }

    pub(super) fn poll_transcription_probe(&self) -> Result<(), String> {
        let result = {
            let probe = self.ivars().transcription_probe.borrow();
            let Some(probe) = probe.as_ref() else {
                return Ok(());
            };
            match probe.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some(Err("Transcription model lookup stopped unexpectedly".into()))
                }
            }
        };
        let Some(result) = result else {
            return Ok(());
        };
        let probe = self
            .ivars()
            .transcription_probe
            .borrow_mut()
            .take()
            .expect("completed transcription probe exists");
        close_alert(&probe.alert);
        self.present_transcription_dialog(probe.server_url, result?)
    }

    fn present_transcription_dialog(
        &self,
        server_url: String,
        models: Vec<String>,
    ) -> Result<(), String> {
        const WIDTH: f64 = 360.0;
        const ROW_HEIGHT: f64 = 32.0;
        const LABEL_WIDTH: f64 = 190.0;
        const CONTROL_HEIGHT: f64 = 26.0;
        const LABEL_Y_OFFSET: f64 = 5.0;
        const ROWS: usize = 5;
        let preferences =
            shrimply_editor_state::preferences::snapshot(&self.ivars().session.preferences);
        let accessory = NSView::initWithFrame(
            NSView::alloc(self.mtm()),
            NSRect::new(NSPoint::ZERO, NSSize::new(WIDTH, ROW_HEIGHT * ROWS as f64)),
        );
        let control_frame = |row: usize| {
            NSRect::new(
                NSPoint::new(LABEL_WIDTH, ROW_HEIGHT * (ROWS - row - 1) as f64),
                NSSize::new(WIDTH - LABEL_WIDTH, CONTROL_HEIGHT),
            )
        };
        let label = |title: &str, row: usize| {
            let label = NSTextField::labelWithString(&NSString::from_str(title), self.mtm());
            let mut frame = control_frame(row);
            frame.origin.x = 0.0;
            frame.origin.y += LABEL_Y_OFFSET;
            frame.size.width = LABEL_WIDTH;
            label.setFrame(frame);
            accessory.addSubview(&label);
        };

        label("Model", 0);
        let model = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(self.mtm()),
            control_frame(0),
            false,
        );
        for candidate in &models {
            model.addItemWithTitle(&NSString::from_str(candidate));
        }
        if let Some(index) = models
            .iter()
            .position(|candidate| candidate == &preferences.last_stt_model)
        {
            model.selectItemAtIndex(index as isize);
        }
        accessory.addSubview(&model);

        label("Audio chunks", 1);
        let chunks = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(self.mtm()),
            control_frame(1),
            false,
        );
        chunks.addItemWithTitle(ns_string!("Follow cuts"));
        chunks.addItemWithTitle(ns_string!("Continuous"));
        accessory.addSubview(&chunks);

        label("Snap source", 2);
        let snap = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(self.mtm()),
            control_frame(2),
            false,
        );
        for title in ["Audio cuts", "Video cuts", "Audio and video cuts"] {
            snap.addItemWithTitle(&NSString::from_str(title));
        }
        accessory.addSubview(&snap);

        label("Snap tolerance (seconds)", 3);
        let tolerance =
            NSTextField::initWithFrame(NSTextField::alloc(self.mtm()), control_frame(3));
        tolerance.setDoubleValue(1.0);
        accessory.addSubview(&tolerance);

        label("Continue threshold (seconds)", 4);
        let threshold =
            NSTextField::initWithFrame(NSTextField::alloc(self.mtm()), control_frame(4));
        threshold.setDoubleValue(2.0);
        accessory.addSubview(&threshold);

        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(ns_string!("Transcribe"));
        let default_chunks =
            match &*self.ivars().content.borrow() {
                Content::Timeline(scene) => scene
                    .transcription_chunk_count(
                        shrimply_timeline_skia::transcription::Options::default(),
                    )
                    .ok(),
                _ => None,
            };
        alert.setInformativeText(&NSString::from_str(&default_chunks.map_or_else(
            || "Create captions from the selected audio.".into(),
            |count| format!("{count} audio chunk(s) will be transcribed."),
        )));
        alert.setAccessoryView(Some(&accessory));
        alert.addButtonWithTitle(ns_string!("Transcribe"));
        alert.addButtonWithTitle(ns_string!("Cancel"));
        if alert.runModal() != NSAlertFirstButtonReturn {
            return Ok(());
        }
        let tolerance = tolerance.doubleValue();
        let threshold = threshold.doubleValue();
        if !tolerance.is_finite() || !(0.0..=2.0).contains(&tolerance) {
            return Err("Snap tolerance must be between 0 and 2 seconds".into());
        }
        if !threshold.is_finite() || !(0.0..=10.0).contains(&threshold) {
            return Err("Continue cut threshold must be between 0 and 10 seconds".into());
        }
        let model_id = models
            .get(
                usize::try_from(model.indexOfSelectedItem())
                    .map_err(|_| "No speech-to-text model is selected")?,
            )
            .cloned()
            .ok_or("No speech-to-text model is selected")?;
        let snap_source = match snap.indexOfSelectedItem() {
            0 => shrimply_timeline_skia::transcription::SnapSource::Audio,
            1 => shrimply_timeline_skia::transcription::SnapSource::Video,
            2 => shrimply_timeline_skia::transcription::SnapSource::AudioAndVideo,
            _ => return Err("Unknown transcription snap source".into()),
        };
        let handle = {
            let mut content = self.ivars().content.borrow_mut();
            let Content::Timeline(scene) = &mut *content else {
                return Err("timeline closed while starting transcription".into());
            };
            scene.start_transcription(
                shrimply_timeline_skia::transcription::Options {
                    chunked: chunks.indexOfSelectedItem() == 0,
                    snap_source,
                    snap_tolerance: shrimply_project_document::project::Time::from_seconds_f64(
                        tolerance,
                    ),
                    continue_threshold: shrimply_project_document::project::Time::from_seconds_f64(
                        threshold,
                    ),
                },
                server_url,
                model_id.clone(),
            )?
        };
        shrimply_editor_state::preferences::set_last_stt_model(
            &self.ivars().session.preferences,
            &model_id,
        );
        let progress = NSProgressIndicator::initWithFrame(
            NSProgressIndicator::alloc(self.mtm()),
            NSRect::new(NSPoint::ZERO, NSSize::new(WIDTH, CONTROL_HEIGHT)),
        );
        progress.setStyle(NSProgressIndicatorStyle::Spinning);
        unsafe { progress.startAnimation(None) };
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(ns_string!("Transcribing"));
        alert.setInformativeText(&NSString::from_str(&format!(
            "{model_id} · Sending request…"
        )));
        alert.setAccessoryView(Some(&progress));
        alert.addButtonWithTitle(ns_string!("Cancel"));
        let completion = RcBlock::new(move |response| {
            if response == NSAlertFirstButtonReturn {
                handle.cancel();
            }
        });
        self.ivars()
            .transcription_alert
            .replace(Some(alert.clone()));
        alert.beginSheetModalForWindow_completionHandler(
            &self.window().expect("canvas must be attached"),
            Some(&completion),
        );
        Ok(())
    }

    pub(super) fn handle_transcription_update(
        &self,
        update: shrimply_timeline_skia::transcription::Update,
    ) -> Result<(), String> {
        match update {
            shrimply_timeline_skia::transcription::Update::Progress(message) => {
                if let Some(alert) = self.ivars().transcription_alert.borrow().as_ref() {
                    alert.setInformativeText(&NSString::from_str(&message));
                }
                return Ok(());
            }
            shrimply_timeline_skia::transcription::Update::Failed(error) => {
                self.close_transcription_alert();
                return Err(error);
            }
            shrimply_timeline_skia::transcription::Update::Finished { .. }
            | shrimply_timeline_skia::transcription::Update::Cancelled => {}
        }
        self.close_transcription_alert();
        Ok(())
    }

    fn close_transcription_alert(&self) {
        let Some(alert) = self.ivars().transcription_alert.borrow_mut().take() else {
            return;
        };
        close_alert(&alert);
    }
}

fn close_alert(alert: &NSAlert) {
    if let Some(parent) = alert.window().sheetParent() {
        parent.endSheet_returnCode(&alert.window(), NSModalResponseStop);
    }
}
