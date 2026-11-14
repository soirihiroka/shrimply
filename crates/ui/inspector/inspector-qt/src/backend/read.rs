use super::*;

impl qobject::InspectorBackend {
    pub fn poll_analysis_control(mut self: Pin<&mut Self>, category: i32, item: i32, control: i32) {
        let Some(key) = analysis_control_key(category, item, control) else {
            return;
        };
        let next = self
            .control_target(category, item, control)
            .and_then(|(target, control)| {
                (control.kind == ControlKind::Analysis).then_some((target, control))
            })
            .and_then(|(target, control)| {
                Some(CachedAnalysisControl {
                    target: target.clone(),
                    action: control.action?,
                    presentation: analysis_presentation(&control, Some(&target))?,
                })
            });
        let (changed, camera_changed, refresh_analysis_output) = match next {
            Some(next) => {
                let previous = self.rust().analysis_controls.get(&key);
                let refresh_analysis_output = crate::with_controller(|controller| {
                    Ok(controller.observe_analysis_transition(
                        &next.target,
                        next.action,
                        &next.presentation,
                    ))
                })
                .expect("Qt analysis control polled before inspector installation");
                let changed = previous != Some(&next);
                let camera_changed = changed
                    && next.action
                        == shrimply_inspector_core::InspectorControlAction::ToggleCameraAnalysis;
                if changed {
                    self.as_mut().rust_mut().analysis_controls.insert(key, next);
                }
                (changed, camera_changed, refresh_analysis_output)
            }
            None => (
                self.as_mut()
                    .rust_mut()
                    .analysis_controls
                    .remove(&key)
                    .is_some(),
                false,
                false,
            ),
        };
        if changed {
            let revision = self.analysis_revision().wrapping_add(1);
            self.as_mut().set_analysis_revision(revision);
        }
        if camera_changed {
            crate::mark_dirty();
        }
        if refresh_analysis_output {
            crate::with_controller(|controller| {
                controller.refresh_analysis_output();
                Ok(())
            })
            .expect("Qt analysis control polled before inspector installation");
        }
    }

    pub fn minimum_width(&self) -> i32 {
        shrimply_inspector_core::INSPECTOR_MIN_WIDTH
    }

    pub fn poll(mut self: Pin<&mut Self>, scroll_position: f64) {
        let (font_browser_changed, font_edits) = crate::receive_font_browser();
        for (edit, activation) in font_edits {
            let result = activation.and_then(|()| crate::apply_font_browser_edit(&edit));
            self.as_mut().finish(result);
        }
        if font_browser_changed {
            let revision = self.font_browser_revision().wrapping_add(1);
            self.as_mut().set_font_browser_revision(revision);
        }
        if self.target_change_pending() {
            crate::mark_dirty();
        }
        let generating = self
            .document()
            .and_then(|document| crate::video_stabilization_generating(&document.target));
        let previous = self.rust().stabilization_generating;
        self.as_mut().rust_mut().stabilization_generating = generating;
        if previous.is_some() && previous != generating {
            crate::mark_dirty();
        }
        let document = crate::take_document();
        let cache_dirty = crate::take_cache_dirty();
        let expression_dirty = crate::take_expression_dirty();
        let graph_dirty = crate::take_graph_dirty();
        let playhead_dirty = crate::take_playhead_dirty();
        let transform_dirty = crate::take_transform_dirty();
        let focus_dirty = crate::take_focus_dirty();
        let Some(document) = document else {
            if cache_dirty {
                let revision = self.cache_revision().wrapping_add(1);
                self.as_mut().set_cache_revision(revision);
            }
            if expression_dirty {
                let revision = self.expression_revision().wrapping_add(1);
                self.as_mut().set_expression_revision(revision);
            }
            if graph_dirty {
                let target = self.document().map(|document| document.target.clone());
                if let Some(target) = target
                    && let Some(document) = self.as_mut().rust_mut().document.as_mut()
                {
                    crate::graph_backend::update_control_graphs(document, &target);
                }
                let revision = self.graph_revision().wrapping_add(1);
                self.as_mut().set_graph_revision(revision);
            }
            let transform_active = self.transform_active();
            if (playhead_dirty || transform_dirty) && transform_active {
                let target = self.document().map(|document| document.target.clone());
                let live = target.as_ref().and_then(crate::transform_live_presentation);
                self.as_mut().rust_mut().resolved_transform = live
                    .as_ref()
                    .map(|presentation| presentation.resolved)
                    .or_else(|| target.as_ref().and_then(crate::resolved_transform));
                if let Some(live) = &live
                    && let Some(document) = self.as_mut().rust_mut().document.as_mut()
                {
                    crate::graph_backend::update_transform_graphs(document, live);
                }
                self.as_mut().rust_mut().transform_live = live;
                let revision = self.transform_revision().wrapping_add(1);
                self.as_mut().set_transform_revision(revision);
            }
            if playhead_dirty {
                let revision = self.playhead_revision().wrapping_add(1);
                self.as_mut().set_playhead_revision(revision);
            }
            if focus_dirty {
                let revision = self.revision().wrapping_add(1);
                self.as_mut().set_revision(revision);
            }
            return;
        };
        if let Some(target) = self.document().map(|document| document.target.clone()) {
            self.as_mut()
                .rust_mut()
                .list_state
                .set_scroll_position(&target, scroll_position);
        }
        let remembered = self.rust().list_state.active_category(&document.target);
        let active = document
            .categories
            .iter()
            .position(|category| remembered == Some(category.key))
            .unwrap_or_default();
        let scroll_position = self.rust().list_state.scroll_position(&document.target);
        let active = i32::try_from(active).expect("inspector category index exceeds Qt limits");
        let title = QString::from(document.title.as_str());
        let analysis_controls = analysis_control_cache(&document);
        let refresh_analysis_output = crate::with_controller(|controller| {
            let mut refresh = false;
            for control in analysis_controls.values() {
                refresh |= controller.observe_analysis_transition(
                    &control.target,
                    control.action,
                    &control.presentation,
                );
            }
            Ok(refresh)
        })
        .expect("Qt analysis document rebuilt before inspector installation");
        let analysis_changed = self.rust().analysis_controls != analysis_controls;
        let transform_live = crate::transform_live_presentation(&document.target);
        let resolved_transform = transform_live
            .as_ref()
            .map(|presentation| presentation.resolved)
            .or_else(|| crate::resolved_transform(&document.target));
        let revision = self.revision().wrapping_add(1);
        let document_revision = self.document_revision().wrapping_add(1);
        self.as_mut().rust_mut().document = Some(document);
        self.as_mut().rust_mut().analysis_controls = analysis_controls;
        self.as_mut().rust_mut().resolved_transform = resolved_transform;
        self.as_mut().rust_mut().transform_live = transform_live;
        self.as_mut().set_ready(true);
        self.as_mut().set_title(title);
        self.as_mut().set_active_category(active);
        self.as_mut().set_scroll_position(scroll_position);
        self.as_mut().set_document_revision(document_revision);
        if analysis_changed {
            let analysis_revision = self.analysis_revision().wrapping_add(1);
            self.as_mut().set_analysis_revision(analysis_revision);
        }
        self.as_mut().set_revision(revision);
        if refresh_analysis_output {
            crate::with_controller(|controller| {
                controller.refresh_analysis_output();
                Ok(())
            })
            .expect("Qt analysis document rebuilt before inspector installation");
        }
    }

    pub fn destructive_background(&self) -> QColor {
        let color = shrimply_cross_ui_theme::current().destructive_bg;
        QColor::from_rgba_f(color.r, color.g, color.b, color.a)
    }

    pub fn target_change_pending(&self) -> bool {
        self.document()
            .is_some_and(|document| crate::target_change_pending(&document.target))
    }

    pub fn destructive_foreground(&self) -> QColor {
        let color = shrimply_cross_ui_theme::current().destructive_fg;
        QColor::from_rgba_f(color.r, color.g, color.b, color.a)
    }

    pub fn keyframe_clipboard_marker(&self) -> QString {
        QString::from(shrimply_inspector_core::keyframe_model::KEYFRAME_CLIPBOARD_MARKER)
    }

    pub fn keyframe_snapping_enabled(&self) -> bool {
        crate::keyframe_snapping().0
    }

    pub fn keyframe_snapping_radius(&self) -> f64 {
        crate::keyframe_snapping().1
    }

    pub fn category_keys(&self) -> QStringList {
        let Some(document) = self.document() else {
            return QStringList::default();
        };
        document
            .categories
            .iter()
            .map(|category| QString::from(format!("{:?}:{}", document.target, category.key)))
            .collect()
    }

    pub fn category_label(&self, category: i32) -> QString {
        self.category(category)
            .map_or_else(QString::default, |category| {
                shrimply_i18n_qt::text(category.label)
            })
    }

    pub fn category_icon(&self, category: i32) -> QString {
        self.category(category)
            .map_or_else(QString::default, |category| {
                QString::from(category.icon.symbolic_name())
            })
    }

    pub fn activate_category(mut self: Pin<&mut Self>, category: i32) {
        let Some(category_value) = self.category(category) else {
            return;
        };
        let key = category_value.key.to_string();
        let target = self
            .document()
            .expect("category has a document")
            .target
            .clone();
        self.as_mut()
            .rust_mut()
            .list_state
            .set_active_category(&target, &key);
        self.as_mut().set_active_category(category);
        if key == "visual" {
            crate::mark_transform_dirty();
        }
    }

    pub fn item_keys(&self, category: i32) -> QStringList {
        let Some(document) = self.document() else {
            return QStringList::default();
        };
        let Some(category) = index(category).and_then(|index| document.categories.get(index))
        else {
            return QStringList::default();
        };
        category
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let key = match item {
                    InspectorListItem::Item(item) => item.presentation.key.as_str(),
                    InspectorListItem::Flat(_) => "flat",
                };
                QString::from(format!(
                    "{:?}:{}:{index}:{key}",
                    document.target, category.key
                ))
            })
            .collect()
    }

    pub fn item_is_card(&self, category: i32, item: i32) -> bool {
        matches!(self.item(category, item), Some(InspectorListItem::Item(_)))
    }

    pub fn item_title(&self, category: i32, item: i32) -> QString {
        self.card(category, item)
            .map_or_else(QString::default, |item| {
                shrimply_i18n_qt::text(&item.presentation.title)
            })
    }

    pub fn item_focus_available(&self, category: i32, item: i32) -> bool {
        self.document().is_some_and(|document| {
            document.preview_item.is_some() && self.card(category, item).is_some()
        })
    }

    pub fn item_focused(&self, category: i32, item: i32) -> bool {
        self.document()
            .zip(self.card(category, item))
            .is_some_and(|(document, item)| crate::item_focused(document, item))
    }

    pub fn focus_item(self: Pin<&mut Self>, category: i32, item: i32) {
        if let Some((document, item)) = self.document().zip(self.card(category, item)) {
            crate::focus_item(document, item);
        }
    }

    pub fn focus_item_body(self: Pin<&mut Self>, category: i32, item: i32) {
        if let Some((document, item)) = self.document().zip(self.card(category, item)) {
            if let Some(control) = item
                .section
                .controls
                .iter()
                .find(|control| control.preview_focus.is_some())
            {
                crate::focus_control(document, item, control);
            } else {
                crate::focus_item(document, item);
            }
        }
    }

    pub fn focus_control(self: Pin<&mut Self>, category: i32, item: i32, control: i32) {
        if let Some((document, item, control)) = self
            .document()
            .zip(self.card(category, item))
            .zip(self.control(category, item, control))
            .map(|((document, item), control)| (document, item, control))
        {
            crate::focus_control(document, item, control);
        }
    }

    pub fn item_expanded(&self, category: i32, item: i32) -> bool {
        let Some(card) = self.card(category, item) else {
            return true;
        };
        let Some(document) = self.document() else {
            return false;
        };
        self.rust()
            .list_state
            .expanded(&document.target, &card.presentation.key)
    }

    pub fn item_identity(&self, category: i32, item: i32) -> QString {
        let Some(document) = self.document() else {
            return QString::default();
        };
        let Some(category) = index(category).and_then(|index| document.categories.get(index))
        else {
            return QString::default();
        };
        let Some(InspectorListItem::Item(item)) =
            index(item).and_then(|index| category.items.get(index))
        else {
            return QString::default();
        };
        QString::from(format!(
            "{:?}:{}:{}",
            document.target, category.key, item.presentation.key
        ))
    }

    pub fn set_item_expanded(mut self: Pin<&mut Self>, category: i32, item: i32, expanded: bool) {
        let Some(card) = self.card(category, item) else {
            return;
        };
        let key = card.presentation.key.clone();
        let transform = crate::graph_backend::has_transform_controls(&card.section);
        let target = self.document().expect("card has a document").target.clone();
        if self.rust().list_state.expanded(&target, &key) == expanded {
            return;
        }
        self.as_mut()
            .rust_mut()
            .list_state
            .set_expanded(&target, &key, expanded);
        if expanded && transform {
            crate::mark_transform_dirty();
        }
        let revision = self.revision().wrapping_add(1);
        self.as_mut().set_revision(revision);
    }

    pub fn item_reset_available(&self, category: i32, item: i32) -> bool {
        self.card(category, item)
            .is_some_and(|item| item.reset.is_some())
    }

    pub fn reset_item(mut self: Pin<&mut Self>, category: i32, item: i32) {
        let action = self
            .card(category, item)
            .and_then(|item| item.reset.clone());
        self.as_mut().perform(action);
    }

    pub fn item_has_toggle(&self, category: i32, item: i32) -> bool {
        self.card(category, item)
            .is_some_and(|item| item.toggle.is_some())
    }

    pub fn item_toggle_active(&self, category: i32, item: i32) -> bool {
        self.card(category, item)
            .and_then(|item| item.toggle.as_ref())
            .is_some_and(|toggle| toggle.active)
    }

    pub fn item_toggle_tooltip(&self, category: i32, item: i32) -> QString {
        self.card(category, item)
            .and_then(|item| item.toggle.as_ref())
            .map_or_else(QString::default, |toggle| {
                shrimply_i18n_qt::text(toggle.tooltip)
            })
    }

    pub fn set_item_toggle(mut self: Pin<&mut Self>, category: i32, item: i32, active: bool) {
        let action = self
            .card(category, item)
            .and_then(|item| item.toggle.as_ref())
            .map(|toggle| boolean_action(toggle.activate.clone(), active));
        self.as_mut().perform(action);
    }

    pub fn item_has_button_toggle(&self, category: i32, item: i32) -> bool {
        self.card(category, item)
            .is_some_and(|item| item.button_toggle.is_some())
    }

    pub fn item_button_toggle_active(&self, category: i32, item: i32) -> bool {
        self.card(category, item)
            .and_then(|item| item.button_toggle.as_ref())
            .is_some_and(|toggle| toggle.active)
    }

    pub fn item_button_toggle_icon(&self, category: i32, item: i32) -> QString {
        self.card(category, item)
            .and_then(|item| item.button_toggle.as_ref())
            .map_or_else(QString::default, |toggle| QString::from(toggle.icon))
    }

    pub fn item_button_toggle_tooltip(&self, category: i32, item: i32) -> QString {
        self.card(category, item)
            .and_then(|item| item.button_toggle.as_ref())
            .map_or_else(QString::default, |toggle| {
                shrimply_i18n_qt::text(toggle.tooltip)
            })
    }

    pub fn set_item_button_toggle(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        active: bool,
    ) {
        let action = self
            .card(category, item)
            .and_then(|item| item.button_toggle.as_ref())
            .map(|toggle| toggle.activate.action(active));
        self.as_mut().perform(action);
    }

    pub fn item_action_count(&self, category: i32, item: i32) -> i32 {
        count(self.card(category, item).map(|item| item.actions.len()))
    }

    pub fn item_action_icon(&self, category: i32, item: i32, action: i32) -> QString {
        self.action(category, item, action)
            .map_or_else(QString::default, |action| QString::from(action.icon))
    }

    pub fn item_action_tooltip(&self, category: i32, item: i32, action: i32) -> QString {
        self.action(category, item, action)
            .map_or_else(QString::default, |action| {
                shrimply_i18n_qt::text(action.tooltip)
            })
    }

    pub fn item_action_sensitive(&self, category: i32, item: i32, action: i32) -> bool {
        self.action(category, item, action)
            .is_some_and(|action| action.sensitive)
    }

    pub fn trigger_item_action(mut self: Pin<&mut Self>, category: i32, item: i32, action: i32) {
        let action = self
            .action(category, item, action)
            .map(|action| action.activate.clone());
        self.as_mut().perform(action);
    }

    pub fn control_count(&self, category: i32, item: i32) -> i32 {
        count(
            self.section(category, item)
                .map(|section| section.controls.len()),
        )
    }

    pub fn control_kind(
        &self,
        category: i32,
        item: i32,
        control: i32,
    ) -> qobject::InspectorControlKind {
        use qobject::InspectorControlKind as QtKind;
        self.control(category, item, control)
            .map_or(QtKind::ReadOnly, |control| match control.kind {
                ControlKind::Boolean => QtKind::Boolean,
                ControlKind::Number => QtKind::Number,
                ControlKind::Fraction => QtKind::Fraction,
                ControlKind::Text => QtKind::Text,
                ControlKind::MultilineText => QtKind::MultilineText,
                ControlKind::ReadOnly => QtKind::ReadOnly,
                ControlKind::Color => QtKind::Color,
                ControlKind::LayeredColor => QtKind::LayeredColor,
                ControlKind::LayeredText => QtKind::LayeredText,
                ControlKind::LayeredDrawing => QtKind::LayeredDrawing,
                ControlKind::FontFamilies => QtKind::FontFamilies,
                ControlKind::VoiceModel
                | ControlKind::Selector
                | ControlKind::OptionalSelector
                | ControlKind::OptionalNumberSelector => QtKind::Selector,
                ControlKind::Vector2 => QtKind::Vector2,
                ControlKind::Vector3 => QtKind::Vector3,
                ControlKind::LayeredNumber => QtKind::LayeredNumber,
                ControlKind::LayeredVector2 => QtKind::LayeredVector2,
                ControlKind::LayeredVector3 => QtKind::LayeredVector3,
                ControlKind::ProjectSettings => QtKind::ProjectSettings,
                ControlKind::Performance => QtKind::Performance,
                ControlKind::LayeredBoolean => QtKind::LayeredBoolean,
                ControlKind::LayeredSelector => QtKind::LayeredSelector,
                ControlKind::AudioCache => QtKind::AudioCache,
                ControlKind::AudioCachePreset => QtKind::AudioCachePreset,
                ControlKind::VisualCache => QtKind::VisualCache,
                ControlKind::VisualCacheQuality => QtKind::VisualCacheQuality,
                ControlKind::Analysis => QtKind::Analysis,
                ControlKind::AudioModifierMenu | ControlKind::VisualModifierMenu => {
                    QtKind::ModifierMenu
                }
                ControlKind::TtsEditor => QtKind::TtsEditor,
                ControlKind::BeatDetection => QtKind::BeatDetection,
                ControlKind::InfoHeading => QtKind::InfoHeading,
                ControlKind::InfoArtwork => QtKind::InfoArtwork,
                ControlKind::FileLocation => QtKind::FileLocation,
                ControlKind::InfoLoading => QtKind::InfoLoading,
                ControlKind::Action => QtKind::Action,
            })
    }

    pub fn control_row_role(
        &self,
        category: i32,
        item: i32,
        control: i32,
    ) -> qobject::InspectorControlRowRole {
        use qobject::InspectorControlRowRole as QtRole;
        self.control(category, item, control)
            .map_or(QtRole::Standalone, |control| match control.row_role {
                ControlRowRole::Standalone => QtRole::Standalone,
                ControlRowRole::Primary => QtRole::Primary,
                ControlRowRole::Auxiliary => QtRole::Auxiliary,
                ControlRowRole::TrailingAction => QtRole::TrailingAction,
            })
    }

    pub fn control_row_member(
        &self,
        category: i32,
        item: i32,
        control: i32,
        role: qobject::InspectorControlRowRole,
    ) -> i32 {
        use qobject::InspectorControlRowRole as QtRole;
        let role = match role {
            QtRole::Standalone => ControlRowRole::Standalone,
            QtRole::Primary => ControlRowRole::Primary,
            QtRole::Auxiliary => ControlRowRole::Auxiliary,
            QtRole::TrailingAction => ControlRowRole::TrailingAction,
            _ => panic!("Qt passed an invalid inspector control row role"),
        };
        let Some((section, group)) = self.section(category, item).zip(
            self.control(category, item, control)
                .and_then(|control| control.row_group),
        ) else {
            return MISSING_CONTROL_INDEX;
        };
        section
            .controls
            .iter()
            .position(|control| control.row_group == Some(group) && control.row_role == role)
            .and_then(|index| i32::try_from(index).ok())
            .unwrap_or(MISSING_CONTROL_INDEX)
    }

    pub fn control_label(&self, category: i32, item: i32, control: i32) -> QString {
        self.control(category, item, control)
            .map_or_else(QString::default, |control| QString::from(&control.label))
    }

    pub fn control_transform_live(&self, category: i32, item: i32, control: i32) -> bool {
        self.control(category, item, control)
            .is_some_and(|control| crate::graph_backend::is_transform_path(&control.path))
    }

    pub fn control_subtitle(&self, category: i32, item: i32, control: i32) -> QString {
        self.control(category, item, control)
            .map_or_else(QString::default, |control| QString::from(&control.subtitle))
    }

    pub fn control_tooltip(&self, category: i32, item: i32, control: i32) -> QString {
        let control_index = control;
        let target = self.document().map(|document| document.target.clone());
        self.control(category, item, control)
            .map_or_else(QString::default, |control| {
                if control.kind == ControlKind::Analysis {
                    return self
                        .cached_analysis_control(category, item, control_index)
                        .map_or_else(
                            || shrimply_i18n_qt::text(&control.tooltip),
                            analysis_tooltip,
                        );
                }
                control.target_id.map_or_else(
                    || shrimply_i18n_qt::text(&control.tooltip),
                    |id| {
                        if matches!(
                            control.kind,
                            ControlKind::AudioCache | ControlKind::VisualCache
                        ) {
                            QString::from(
                                crate::tracked_cache_control(control.kind, id, target.as_ref())
                                    .map_or_else(
                                        || control.tooltip.clone(),
                                        |status| status.tooltip,
                                    ),
                            )
                        } else {
                            shrimply_i18n_qt::text(&control.tooltip)
                        }
                    },
                )
            })
    }

    pub fn control_value(&self, category: i32, item: i32, control: i32) -> QString {
        let control_index = control;
        let target = self.document().map(|document| document.target.clone());
        self.control(category, item, control)
            .map_or_else(QString::default, |control| {
                if let Some(bytes) = &control.image_bytes {
                    use base64::Engine;
                    QString::from(format!(
                        "data:application/octet-stream;base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(bytes)
                    ))
                } else if control.kind == ControlKind::Fraction {
                    QString::from(fraction_value(control).to_string())
                } else if control.kind == ControlKind::LayeredNumber {
                    let cached = self
                        .rust()
                        .transform_live
                        .as_ref()
                        .and_then(|live| live.number(&control.path));
                    let value = cached
                        .or_else(|| {
                            target.as_ref().and_then(|target| {
                                crate::timeline_number_value(
                                    target,
                                    control.audio_modifier,
                                    control.target_id,
                                    control.timeline_id,
                                    control.timeline_path.as_deref().unwrap_or(&control.path),
                                )
                                .ok()
                            })
                        })
                        .map(|value| control.display_number(value))
                        .unwrap_or_else(|| control.value.parse::<f64>().unwrap_or_default());
                    QString::from(value.to_string())
                } else if matches!(
                    control.kind,
                    ControlKind::AudioCache | ControlKind::VisualCache
                ) {
                    control
                        .target_id
                        .and_then(|id| {
                            crate::tracked_cache_control(control.kind, id, target.as_ref())
                        })
                        .map_or_else(
                            || shrimply_i18n_qt::text(&control.value),
                            |status| shrimply_i18n_qt::text(status.label),
                        )
                } else if control.kind == ControlKind::Analysis {
                    QString::from(
                        self.cached_analysis_control(category, item, control_index)
                            .map_or(control.value.as_str(), |status| status.label.as_str()),
                    )
                } else {
                    QString::from(control.value.as_str())
                }
            })
    }

    pub fn control_component(&self, category: i32, item: i32, control: i32, component: i32) -> f64 {
        let control_index = control;
        let target = self.document().map(|document| document.target.clone());
        self.control(category, item, control)
            .and_then(|control| {
                if control.kind == ControlKind::Analysis && component >= 0 {
                    let status = self.cached_analysis_control(category, item, control_index)?;
                    return match component {
                        0 => Some(status.progress),
                        1 => Some(f64::from(u8::from(status.running))),
                        2 => Some(f64::from(u8::from(status.cancelling))),
                        3 => Some(f64::from(u8::from(status.suggested))),
                        _ => None,
                    };
                }
                if matches!(
                    control.kind,
                    ControlKind::AudioCache | ControlKind::VisualCache
                ) {
                    let status = control
                        .target_id
                        .and_then(|id| {
                            crate::tracked_cache_control(control.kind, id, target.as_ref())
                        })
                        .and_then(|status| match component {
                            0 => Some(status.progress),
                            1 => Some(f64::from(u8::from(status.baking))),
                            _ => None,
                        });
                    return status.or_else(|| {
                        control
                            .components
                            .get(usize::try_from(component).ok()?)?
                            .parse()
                            .ok()
                    });
                }
                if control.kind == ControlKind::LayeredVector2 {
                    let value = self
                        .rust()
                        .transform_live
                        .as_ref()
                        .and_then(|live| live.vector(&control.path))
                        .or_else(|| {
                            let transform = self.rust().resolved_transform?;
                            match control.path.as_str() {
                                "/transform/position" => Some(transform.position),
                                "/transform/anchor" => Some(transform.anchor),
                                "/transform/scale" => Some(transform.scale),
                                "/transform/shear" => Some(transform.shear),
                                _ => None,
                            }
                        })
                        .or_else(|| {
                            target.as_ref().and_then(|target| {
                                crate::timeline_vector2_value(
                                    target,
                                    control.timeline_id,
                                    control.timeline_path.as_deref().unwrap_or(&control.path),
                                )
                                .ok()
                            })
                        })
                        .or_else(|| {
                            Some(glam::Vec2::new(
                                control.components.first()?.parse().ok()?,
                                control.components.get(1)?.parse().ok()?,
                            ))
                        })?;
                    return match component {
                        0 => Some(f64::from(value.x)),
                        1 => Some(f64::from(value.y)),
                        _ => None,
                    };
                }
                if control.kind == ControlKind::LayeredVector3 {
                    let timeline_id = control.timeline_id?;
                    let value = target
                        .as_ref()
                        .and_then(|target| {
                            crate::timeline_vector3_value(
                                target,
                                timeline_id,
                                control.timeline_path.as_deref().unwrap_or(&control.path),
                            )
                            .ok()
                        })
                        .or_else(|| {
                            Some(glam::Vec3::new(
                                control.components.first()?.parse().ok()?,
                                control.components.get(1)?.parse().ok()?,
                                control.components.get(2)?.parse().ok()?,
                            ))
                        })?;
                    return match component {
                        0 => Some(f64::from(value.x)),
                        1 => Some(f64::from(value.y)),
                        2 => Some(f64::from(value.z)),
                        _ => None,
                    };
                }
                control.components.get(index(component)?)?.parse().ok()
            })
            .unwrap_or_default()
    }

    pub fn control_color(&self, category: i32, item: i32, control: i32) -> QStringList {
        let Some((target, control)) = self.control_target(category, item, control) else {
            return QStringList::default();
        };
        if control.kind != ControlKind::LayeredColor {
            return QStringList::default();
        }
        let Some(timeline_id) = control.timeline_id else {
            return QStringList::default();
        };
        crate::color_value(&target, &control.path, timeline_id)
            .map(|color| [color.r, color.g, color.b, color.a])
            .unwrap_or_default()
            .into_iter()
            .map(|channel| QString::from(channel.to_string()))
            .collect()
    }

    pub fn control_component_text(
        &self,
        category: i32,
        item: i32,
        control: i32,
        component: i32,
    ) -> QString {
        self.control(category, item, control)
            .and_then(|control| control.components.get(index(component)?))
            .map_or_else(QString::default, QString::from)
    }

    pub fn control_editable(&self, category: i32, item: i32, control: i32) -> bool {
        self.control(category, item, control)
            .is_some_and(|control| control.editable)
    }

    pub fn control_sensitive(&self, category: i32, item: i32, control: i32) -> bool {
        let control_index = control;
        let target = self.document().map(|document| document.target.clone());
        self.control(category, item, control)
            .is_some_and(|control| {
                if control.kind == ControlKind::Analysis {
                    return self
                        .cached_analysis_control(category, item, control_index)
                        .map_or(control.sensitive, |status| status.sensitive);
                }
                control.sensitive
                    && !(matches!(
                        control.kind,
                        ControlKind::AudioCachePreset | ControlKind::VisualCacheQuality
                    ) && control
                        .target_id
                        .and_then(|id| {
                            crate::tracked_cache_control(control.kind, id, target.as_ref())
                        })
                        .is_some_and(|status| status.baking))
            })
    }

    pub fn control_visible(&self, category: i32, item: i32, control: i32) -> bool {
        self.control(category, item, control)
            .is_some_and(|control| control.visible)
    }

    pub fn control_busy(&self, category: i32, item: i32, control: i32) -> bool {
        let control_index = control;
        self.control(category, item, control)
            .is_some_and(|control| {
                if control.kind == ControlKind::Analysis {
                    return self
                        .cached_analysis_control(category, item, control_index)
                        .map_or(control.busy, |status| status.active());
                }
                control.busy
                    || control.kind == ControlKind::BeatDetection
                        && control
                            .target_id
                            .is_some_and(shrimply_audio_engine::beat::is_loading)
            })
    }

    pub fn show_control_path(mut self: Pin<&mut Self>, category: i32, item: i32, control: i32) {
        let result = self
            .control(category, item, control)
            .filter(|control| control.kind == ControlKind::FileLocation)
            .ok_or_else(|| "inspector control is not a file location".to_string())
            .and_then(|control| {
                shrimply_components_qt::desktop_open::prepare(
                    std::path::Path::new(&control.value),
                    None,
                )
            });
        match result {
            Ok(shrimply_components_qt::desktop_open::Action::Open(path)) => self
                .as_mut()
                .open_path(QUrl::from_local_file(&QString::from(
                    path.to_string_lossy().as_ref(),
                ))),
            Ok(shrimply_components_qt::desktop_open::Action::FocusRevealed(_)) => {}
            Err(error) => self.as_mut().show_error(QString::from(error)),
        }
    }

    pub fn control_minimum(&self, category: i32, item: i32, control: i32) -> f64 {
        self.control(category, item, control)
            .map_or(0.0, |control| control.number.minimum)
    }

    pub fn control_maximum(&self, category: i32, item: i32, control: i32) -> f64 {
        self.control(category, item, control)
            .map_or(0.0, |control| control.number.maximum)
    }

    pub fn control_drag_step(&self, category: i32, item: i32, control: i32) -> f64 {
        self.control(category, item, control)
            .map_or(1.0, |control| control.number.drag_step)
    }

    pub fn control_digits(&self, category: i32, item: i32, control: i32) -> i32 {
        self.control(category, item, control)
            .map_or(2, |control| control.number.digits)
    }

    pub fn control_unit(&self, category: i32, item: i32, control: i32) -> QString {
        self.control(category, item, control)
            .map_or_else(QString::default, |control| {
                QString::from(control.number.unit)
            })
    }

    pub fn control_width_characters(&self, category: i32, item: i32, control: i32) -> i32 {
        self.control(category, item, control)
            .map_or(8, |control| control.width_characters)
    }

    pub fn control_prefix_icon(&self, category: i32, item: i32, control: i32) -> QString {
        self.control(category, item, control)
            .map_or_else(QString::default, |control| {
                QString::from(control.prefix_icon.as_str())
            })
    }

    pub fn control_has_action(&self, category: i32, item: i32, control: i32) -> bool {
        self.control(category, item, control)
            .is_some_and(|control| control.action.is_some())
    }

    pub fn control_action_icon(&self, category: i32, item: i32, control: i32) -> QString {
        self.control(category, item, control)
            .map_or_else(QString::default, |control| {
                QString::from(control.action_icon.as_str())
            })
    }

    pub fn control_action_sensitive(&self, category: i32, item: i32, control: i32) -> bool {
        self.control(category, item, control)
            .is_some_and(|control| control.action_sensitive)
    }

    pub fn control_action_tooltip(&self, category: i32, item: i32, control: i32) -> QString {
        self.control(category, item, control)
            .map_or_else(QString::default, |control| {
                shrimply_i18n_qt::text(&control.action_tooltip)
            })
    }

    pub fn control_drag_payload(&self, category: i32, item: i32, control: i32) -> QString {
        self.control(category, item, control)
            .map_or_else(QString::default, |control| {
                QString::from(control.drag_payload.as_str())
            })
    }

    pub fn control_prefix_icon_rotates(&self, category: i32, item: i32, control: i32) -> bool {
        self.control(category, item, control)
            .is_some_and(|control| control.prefix_icon_rotates)
    }

    pub fn control_prefix_icon_rotation_offset(
        &self,
        category: i32,
        item: i32,
        control: i32,
    ) -> f64 {
        self.control(category, item, control)
            .map_or(0.0, |control| control.prefix_icon_rotation_offset_degrees)
    }

    pub fn control_prefix(
        &self,
        category: i32,
        item: i32,
        control: i32,
        component: i32,
    ) -> QString {
        let defaults = ["X", "Y", "Z"];
        self.control(category, item, control)
            .and_then(|control| control.prefixes.get(index(component)?))
            .map_or_else(
                || {
                    index(component)
                        .and_then(|component| defaults.get(component))
                        .map_or_else(QString::default, |prefix| QString::from(*prefix))
                },
                |prefix| QString::from(prefix.as_str()),
            )
    }

    pub fn control_lock(&self, category: i32, item: i32, control: i32) -> bool {
        self.control(category, item, control)
            .is_some_and(|control| control.lock)
    }

    pub fn control_with_alpha(&self, category: i32, item: i32, control: i32) -> bool {
        self.control(category, item, control)
            .is_some_and(|control| control.with_alpha)
    }

    pub fn control_choice_values(&self, category: i32, item: i32, control: i32) -> QStringList {
        strings(
            self.control(category, item, control)
                .map(|control| control.values.as_slice()),
        )
    }

    pub fn control_choice_labels(&self, category: i32, item: i32, control: i32) -> QStringList {
        self.control(category, item, control)
            .map(|control| {
                control
                    .labels
                    .iter()
                    .map(|label| shrimply_i18n_qt::text(label))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn control_choice_icons(&self, category: i32, item: i32, control: i32) -> QStringList {
        strings(
            self.control(category, item, control)
                .map(|control| control.icons.as_slice()),
        )
    }

    pub fn control_choice_search_terms(
        &self,
        category: i32,
        item: i32,
        control: i32,
    ) -> QStringList {
        strings(
            self.control(category, item, control)
                .map(|control| control.search_terms.as_slice()),
        )
    }

    pub fn open_font_browser(mut self: Pin<&mut Self>) {
        if crate::open_font_browser() {
            let revision = self.font_browser_revision().wrapping_add(1);
            self.as_mut().set_font_browser_revision(revision);
        }
    }

    pub fn search_font_browser(mut self: Pin<&mut Self>, query: &QString) {
        if crate::search_font_browser(query.to_string()) {
            let revision = self.font_browser_revision().wrapping_add(1);
            self.as_mut().set_font_browser_revision(revision);
        }
    }

    pub fn request_font_browser_previews(self: Pin<&mut Self>, first: i32, count: i32) {
        let (Ok(first), Ok(count)) = (usize::try_from(first), usize::try_from(count)) else {
            return;
        };
        let Some(end) = first.checked_add(count) else {
            return;
        };
        crate::request_font_browser_previews(first..end);
    }

    pub fn font_browser_count(&self) -> i32 {
        count(Some(crate::font_browser_count()))
    }

    pub fn font_browser_label(&self, choice: i32) -> QString {
        index(choice)
            .and_then(crate::font_browser_choice)
            .map_or_else(QString::default, |family| QString::from(&family.name))
    }

    pub fn font_browser_value(&self, choice: i32) -> QString {
        index(choice)
            .and_then(crate::font_browser_choice)
            .map_or_else(QString::default, |family| {
                QString::from(
                    serde_json::to_string(&shrimply_inspector_core::font_cache::project_family(
                        &family,
                    ))
                    .expect("font family must serialize"),
                )
            })
    }

    pub fn font_browser_google(&self, choice: i32) -> bool {
        index(choice)
            .and_then(crate::font_browser_choice)
            .is_some_and(|family| {
                family.source == shrimply_inspector_core::font_cache::FontSource::Google
            })
    }

    pub fn font_browser_preview_source(&self, choice: i32) -> QUrl {
        let Some(family) = index(choice).and_then(crate::font_browser_choice) else {
            return QUrl::default();
        };
        match shrimply_inspector_core::font_cache::preview_source(
            &family,
            crate::font_browser_lookup().as_ref(),
        ) {
            Ok(shrimply_inspector_core::font_cache::FontPreviewSource::Installed) | Err(_) => {
                QUrl::default()
            }
            Ok(shrimply_inspector_core::font_cache::FontPreviewSource::File(path)) => {
                QUrl::from_local_file(&QString::from(path.to_string_lossy().as_ref()))
            }
            Ok(shrimply_inspector_core::font_cache::FontPreviewSource::Remote(url)) => {
                QUrl::from(url.as_str())
            }
        }
    }

    pub fn font_browser_status(&self) -> QString {
        QString::from(crate::font_browser_status())
    }

    pub fn font_browser_busy(&self) -> bool {
        crate::font_browser_busy()
    }

    pub fn font_list_with_choice(&self, value: &QString, index: i32, family: &QString) -> QString {
        let Ok(families) =
            serde_json::from_str::<Vec<shrimply_property_model::FontFamily>>(&value.to_string())
        else {
            return QString::default();
        };
        let Ok(family) =
            serde_json::from_str::<shrimply_property_model::FontFamily>(&family.to_string())
        else {
            return QString::default();
        };
        let next = if index < 0 {
            shrimply_inspector_core::font_selector::append_family(&families, family)
        } else {
            usize::try_from(index).ok().and_then(|index| {
                shrimply_inspector_core::font_selector::replace_family(&families, index, family)
            })
        };
        next.map_or_else(QString::default, |next| {
            QString::from(serde_json::to_string(&next).expect("font families must serialize"))
        })
    }

    pub fn move_font_list_value(&self, value: &QString, index: i32, offset: i32) -> QString {
        let Ok(families) =
            serde_json::from_str::<Vec<shrimply_property_model::FontFamily>>(&value.to_string())
        else {
            return QString::default();
        };
        let next = usize::try_from(index).ok().and_then(|index| {
            isize::try_from(offset).ok().and_then(|offset| {
                shrimply_inspector_core::font_selector::move_family(&families, index, offset)
            })
        });
        next.map_or_else(QString::default, |next| {
            QString::from(serde_json::to_string(&next).expect("font families must serialize"))
        })
    }

    pub fn remove_font_list_value(&self, value: &QString, index: i32) -> QString {
        let Ok(families) =
            serde_json::from_str::<Vec<shrimply_property_model::FontFamily>>(&value.to_string())
        else {
            return QString::default();
        };
        let next = usize::try_from(index).ok().and_then(|index| {
            shrimply_inspector_core::font_selector::remove_family(&families, index)
        });
        next.map_or_else(QString::default, |next| {
            QString::from(serde_json::to_string(&next).expect("font families must serialize"))
        })
    }

    pub fn activate_control_font(
        mut self: Pin<&mut Self>,
        category: i32,
        item: i32,
        control: i32,
        family: &QString,
        value: &QString,
    ) {
        let result = self
            .control_target(category, item, control)
            .ok_or_else(|| "font control is no longer available".to_string())
            .and_then(|(target, control)| {
                if control.kind != ControlKind::FontFamilies {
                    return Err("inspector control is not a font family list".to_string());
                }
                let chosen: shrimply_property_model::FontFamily =
                    serde_json::from_str(&family.to_string())
                        .map_err(|error| format!("invalid font family: {error}"))?;
                let next: Vec<shrimply_property_model::FontFamily> =
                    serde_json::from_str(&value.to_string())
                        .map_err(|error| format!("invalid font family list: {error}"))?;
                if !next.iter().any(|candidate| candidate == &chosen) {
                    return Err("selected font is missing from the font list".to_string());
                }
                let source = match &chosen {
                    shrimply_property_model::FontFamily::Local { .. } => {
                        shrimply_inspector_core::font_cache::FontSource::Local
                    }
                    shrimply_property_model::FontFamily::GoogleFonts { .. } => {
                        shrimply_inspector_core::font_cache::FontSource::Google
                    }
                };
                let available = crate::find_font_browser_choice(chosen.name(), source)
                    .ok_or_else(|| "selected font is no longer available".to_string())?;
                let modifier_id = control.target_id;
                crate::with_controller(|controller| {
                    crate::ensure_font_control(controller, &target, &control.path, modifier_id)
                })?;
                crate::activate_font_browser_family(
                    available,
                    crate::PendingFontEdit {
                        target,
                        modifier_id,
                        path: control.path,
                        commit_name: control.commit_name,
                        source_value: control.value,
                        value: value.to_string(),
                    },
                )
            });
        if let Err(error) = result {
            self.as_mut().finish(Err(error));
        }
    }

    pub fn cancel_control_font_activation(&self) {
        crate::cancel_font_browser_edit();
    }

    pub fn control_keyframes(&self, category: i32, item: i32, control: i32) -> bool {
        self.control(category, item, control)
            .is_some_and(|control| control.layered.keyframes)
    }

    pub fn control_expression(&self, category: i32, item: i32, control: i32) -> bool {
        self.control(category, item, control)
            .is_some_and(|control| control.layered.expression)
    }

    pub fn control_expression_source(&self, category: i32, item: i32, control: i32) -> QString {
        self.control(category, item, control)
            .map_or_else(QString::default, |control| {
                QString::from(control.layered.expression_source.as_str())
            })
    }

    pub fn control_expression_result(&self, category: i32, item: i32, control: i32) -> QStringList {
        let Some(document) = self.document() else {
            return QStringList::default();
        };
        let Some(control) = self.control(category, item, control) else {
            return QStringList::default();
        };
        if !control.layered.expression {
            return QStringList::default();
        }
        let path = control.timeline_path.as_deref().unwrap_or(&control.path);
        if let Some(field) = shrimply_inspector_core::transform::TransformField::from_path(path) {
            let Some(timeline_id) = control.timeline_id else {
                return QStringList::default();
            };
            let output = match field {
                shrimply_inspector_core::transform::TransformField::Vec2(field) => {
                    let Ok(Some(output)) = crate::transform_vec2_expression_output(
                        &document.target,
                        field,
                        timeline_id,
                    ) else {
                        return QStringList::default();
                    };
                    [
                        shrimply_inspector_core::transform::expressions::format_vec2(
                            field,
                            output.value,
                        ),
                        output.error.unwrap_or_default(),
                    ]
                }
                shrimply_inspector_core::transform::TransformField::Scalar(field) => {
                    let Ok(Some(output)) = crate::transform_scalar_expression_output(
                        &document.target,
                        field,
                        timeline_id,
                    ) else {
                        return QStringList::default();
                    };
                    [
                        shrimply_inspector_core::transform::expressions::format_scalar(
                            field,
                            output.value,
                        ),
                        output.error.unwrap_or_default(),
                    ]
                }
            };
            return output.into_iter().map(QString::from).collect();
        }
        if control.kind == ControlKind::LayeredBoolean {
            let Ok(output) = crate::bool_expression_output(&document.target, path) else {
                return QStringList::default();
            };
            return [output.value.to_string(), output.error.unwrap_or_default()]
                .into_iter()
                .map(QString::from)
                .collect();
        }
        if control.kind == ControlKind::LayeredSelector {
            let Ok(output) =
                crate::step_expression_output(&document.target, path, control.timeline_id)
            else {
                return QStringList::default();
            };
            return [output.value, output.error.unwrap_or_default()]
                .into_iter()
                .map(QString::from)
                .collect();
        }
        if control.kind == ControlKind::LayeredVector2 {
            let Ok(output) =
                crate::vector2_expression_output(&document.target, path, control.timeline_id)
            else {
                return QStringList::default();
            };
            let digits = usize::try_from(control.number.digits).unwrap_or_default();
            let first_prefix = control.prefixes.first().map_or("X", String::as_str);
            let second_prefix = control.prefixes.get(1).map_or("Y", String::as_str);
            return [
                shrimply_inspector_core::timeline_value::vector::vec2::format_value(
                    output.value,
                    first_prefix,
                    second_prefix,
                    digits,
                    control.number.unit,
                ),
                output.error.unwrap_or_default(),
            ]
            .into_iter()
            .map(QString::from)
            .collect();
        }
        if control.kind == ControlKind::LayeredVector3 {
            let Some(timeline_id) = control.timeline_id else {
                return QStringList::default();
            };
            let Ok(output) = crate::vector3_expression_output(&document.target, path, timeline_id)
            else {
                return QStringList::default();
            };
            let digits = usize::try_from(control.number.digits).unwrap_or_default();
            let first_prefix = control.prefixes.first().map_or("X", String::as_str);
            let second_prefix = control.prefixes.get(1).map_or("Y", String::as_str);
            let third_prefix = control.prefixes.get(2).map_or("Z", String::as_str);
            return [
                shrimply_inspector_core::timeline_value::vector::vec3::format_value(
                    output.value,
                    [first_prefix, second_prefix, third_prefix],
                    digits,
                    control.number.unit,
                ),
                output.error.unwrap_or_default(),
            ]
            .into_iter()
            .map(QString::from)
            .collect();
        }
        if control.kind == ControlKind::LayeredColor {
            let Some(timeline_id) = control.timeline_id else {
                return QStringList::default();
            };
            let Ok(output) = crate::color_expression_output(&document.target, path, timeline_id)
            else {
                return QStringList::default();
            };
            return [
                format!(
                    "#{:02X}{:02X}{:02X}{:02X}",
                    output.value.r, output.value.g, output.value.b, output.value.a,
                ),
                output.error.unwrap_or_default(),
            ]
            .into_iter()
            .map(QString::from)
            .collect();
        }
        if control.kind == ControlKind::LayeredText {
            let Some(timeline_id) = control.timeline_id else {
                return QStringList::default();
            };
            let Ok(output) = crate::text_expression_output(&document.target, path, timeline_id)
            else {
                return QStringList::default();
            };
            return [output.value, output.error.unwrap_or_default()]
                .into_iter()
                .map(QString::from)
                .collect();
        }
        if control.kind == ControlKind::LayeredDrawing {
            let Some(timeline_id) = control.timeline_id else {
                return QStringList::default();
            };
            let Ok(output) = crate::paint_drawing_expression_output(&document.target, timeline_id)
            else {
                return QStringList::default();
            };
            return [output.value, output.error.unwrap_or_default()]
                .into_iter()
                .map(QString::from)
                .collect();
        }
        if control.background_integer() {
            let Some(timeline_id) = control.timeline_id else {
                return QStringList::default();
            };
            let Ok(output) =
                crate::background_integer_expression_output(&document.target, path, timeline_id)
            else {
                return QStringList::default();
            };
            return [output.value.to_string(), output.error.unwrap_or_default()]
                .into_iter()
                .map(QString::from)
                .collect();
        }
        let output = if control.audio_modifier {
            control
                .target_id
                .zip(control.timeline_id)
                .ok_or_else(|| "audio modifier expression target is unavailable".to_string())
                .and_then(|(modifier_id, timeline_id)| {
                    crate::audio_modifier_expression_output(
                        &document.target,
                        modifier_id,
                        timeline_id,
                    )
                })
        } else {
            crate::scalar_expression_output(&document.target, path, control.timeline_id)
        };
        let Ok(output) = output else {
            return QStringList::default();
        };
        [
            format!(
                "{:.*}{}",
                usize::try_from(control.number.digits).unwrap_or_default(),
                control.display_number(f64::from(output.value)),
                control.number.unit,
            ),
            output.error.unwrap_or_default(),
        ]
        .into_iter()
        .map(QString::from)
        .collect()
    }

    pub fn expression_diagnostic(mut self: Pin<&mut Self>, source: &QString) -> QStringList {
        let source = source.to_string();
        let diagnostic = self
            .as_mut()
            .rust_mut()
            .expression_diagnostic_cache
            .diagnostic(&source)
            .cloned();
        let Some(diagnostic) = diagnostic else {
            return QStringList::default();
        };
        [
            diagnostic.message,
            diagnostic
                .line
                .map_or_else(String::new, |line| line.to_string()),
            diagnostic
                .column
                .map_or_else(String::new, |column| column.to_string()),
        ]
        .into_iter()
        .map(QString::from)
        .collect()
    }

    pub fn expression_diagnostic_debounce(&self) -> i32 {
        shrimply_inspector_core::rhai_editor::DIAGNOSTIC_DEBOUNCE_MILLISECONDS
    }

    pub fn expression_completion_debounce(&self) -> i32 {
        shrimply_inspector_core::rhai_editor::COMPLETION_DEBOUNCE_MILLISECONDS
    }

    pub fn control_expression_completion(
        &self,
        category: i32,
        item: i32,
        control: i32,
        source: &QString,
        cursor: i32,
        automatic: bool,
    ) -> QStringList {
        let Some(control) = self.control(category, item, control) else {
            return QStringList::default();
        };
        let value = match control.kind {
            ControlKind::LayeredBoolean => {
                shrimply_inspector_core::rhai_editor::ExpressionValue::Bool
            }
            ControlKind::LayeredSelector => {
                shrimply_inspector_core::rhai_editor::ExpressionValue::Step
            }
            ControlKind::LayeredText => shrimply_inspector_core::rhai_editor::ExpressionValue::Text,
            ControlKind::LayeredDrawing => {
                shrimply_inspector_core::rhai_editor::ExpressionValue::Drawing
            }
            ControlKind::LayeredVector2 => {
                shrimply_inspector_core::rhai_editor::ExpressionValue::Vec2
            }
            ControlKind::LayeredVector3 => {
                shrimply_inspector_core::rhai_editor::ExpressionValue::Vec3
            }
            ControlKind::LayeredColor => {
                shrimply_inspector_core::rhai_editor::ExpressionValue::Color
            }
            _ => shrimply_inspector_core::rhai_editor::ExpressionValue::Scalar,
        };
        let source = source.to_string();
        let utf16_cursor = usize::try_from(cursor).unwrap_or_default();
        let cursor = shrimply_inspector_core::rhai_editor::utf16_offset_to_char_offset(
            &source,
            utf16_cursor,
        );
        let completion = if automatic {
            shrimply_inspector_core::rhai_editor::automatic_completion(&source, value, cursor)
        } else {
            shrimply_inspector_core::rhai_editor::completion(&source, value, cursor)
        };
        let Some(completion) = completion else {
            return QStringList::default();
        };
        let start = shrimply_inspector_core::rhai_editor::char_offset_to_utf16_offset(
            &source,
            completion.start,
        );
        let end = shrimply_inspector_core::rhai_editor::char_offset_to_utf16_offset(
            &source,
            completion.end,
        );
        [start.to_string(), end.to_string()]
            .into_iter()
            .chain(completion.candidates)
            .map(QString::from)
            .collect()
    }
}
