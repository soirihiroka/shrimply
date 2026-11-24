use std::{cell::Cell, rc::Rc};

use adw::prelude::*;
use gtk::{gdk, glib};
use shrimply_components_gtk::layout::TRANSPORT_ROW_MIN_WIDTH;

use crate::{player_state, project::Time};
use player_state::{SharedPlayerState, Snapshot};

#[derive(Clone, Copy, PartialEq, Eq)]
enum SeekInput {
    Button(u32),
    Touch(usize),
}

#[derive(Default)]
pub(super) struct PlaybarSeek {
    input: Cell<Option<SeekInput>>,
    pending: Cell<Option<Time>>,
    syncing: Cell<bool>,
}

impl PlaybarSeek {
    pub(super) fn attach(scale: &gtk::Scale, player: &SharedPlayerState) -> Rc<Self> {
        let seek = Rc::new(Self::default());
        let events = gtk::EventControllerLegacy::new();
        events.set_propagation_phase(gtk::PropagationPhase::Capture);
        events.connect_event({
            let seek = seek.clone();
            let player = player.clone();
            let scale = scale.downgrade();
            move |_, event| {
                let kind = event.event_type();
                let input = match kind {
                    gdk::EventType::ButtonPress | gdk::EventType::ButtonRelease => event
                        .downcast_ref::<gdk::ButtonEvent>()
                        .map(|event| SeekInput::Button(event.button())),
                    gdk::EventType::TouchBegin
                    | gdk::EventType::TouchEnd
                    | gdk::EventType::TouchCancel => {
                        Some(SeekInput::Touch(event.event_sequence().as_ptr() as usize))
                    }
                    _ => None,
                };
                if matches!(
                    kind,
                    gdk::EventType::ButtonPress | gdk::EventType::TouchBegin
                ) && seek.input.get().is_none()
                    && input.is_some()
                {
                    seek.input.set(input);
                    player_state::set_scrubbing(&player, true);
                }
                let finishing = match kind {
                    gdk::EventType::ButtonRelease
                    | gdk::EventType::TouchEnd
                    | gdk::EventType::TouchCancel => input == seek.input.get(),
                    gdk::EventType::GrabBroken => true,
                    _ => false,
                };
                if finishing && let Some(scale) = scale.upgrade() {
                    seek.finish(&scale, &player);
                }
                glib::Propagation::Proceed
            }
        });
        scale.add_controller(events);
        scale.connect_unmap({
            let seek = seek.clone();
            let player = player.clone();
            move |scale| seek.finish(scale, &player)
        });
        scale.connect_value_changed({
            let seek = seek.clone();
            let player = player.clone();
            move |scale| {
                if seek.syncing.get() {
                    return;
                }
                let position = Time::from_seconds_f64(scale.value());
                if seek.input.get().is_some() {
                    seek.pending.set(Some(position));
                } else {
                    player_state::seek_time(&player, position);
                }
            }
        });
        seek
    }

    pub(super) fn sync(&self, scale: &gtk::Scale, snapshot: Snapshot) {
        if self.input.get().is_some() {
            return;
        }
        let position = snapshot.position.as_secs_f64();
        self.syncing.set(true);
        scale.set_range(0.0, snapshot.duration.as_secs_f64().max(position).max(0.01));
        scale.set_value(position);
        self.syncing.set(false);
    }

    pub(super) fn flush(&self, player: &SharedPlayerState) {
        if let Some(position) = self.pending.take() {
            player_state::seek_time(player, position);
        }
    }

    fn finish(&self, scale: &gtk::Scale, player: &SharedPlayerState) {
        if self.input.take().is_some() {
            player_state::set_scrubbing(player, false);
            self.flush(player);
            self.sync(scale, player_state::snapshot(player));
        }
    }
}

pub(super) fn controls(
    buttons: [&gtk::Button; 3],
    progress: &gtk::Scale,
    time: &gtk::Label,
    fullscreen: &gtk::Button,
) -> gtk::Box {
    const GAP: i32 = 8;
    const TIME_COLUMNS: i32 = 34;
    let transport = gtk::Box::new(gtk::Orientation::Horizontal, GAP);
    for button in buttons {
        transport.append(button);
    }
    fullscreen.set_hexpand(false);
    let first = gtk::Box::new(gtk::Orientation::Horizontal, GAP);
    let second = gtk::Box::new(gtk::Orientation::Horizontal, GAP);
    let rows = gtk::Box::new(gtk::Orientation::Vertical, GAP);
    rows.append(&first);
    rows.append(&second);
    let widgets = [
        transport.upcast_ref::<gtk::Widget>(),
        progress.upcast_ref(),
        time.upcast_ref(),
        fullscreen.upcast_ref(),
    ];
    for widget in widgets {
        first.append(widget);
    }
    second.set_visible(false);
    let wide_minimum = f64::from(rows.measure(gtk::Orientation::Horizontal, -1).0)
        .max(f64::from(TRANSPORT_ROW_MIN_WIDTH));
    let bin = adw::BreakpointBin::new();
    bin.set_child(Some(&rows));
    let sizing_bin = bin.downgrade();
    let rows = rows.downgrade();
    let first = first.downgrade();
    let second = second.downgrade();
    let widgets = widgets.map(|widget| widget.downgrade());
    let arrange: Rc<dyn Fn(bool)> = Rc::new(move |stacked| {
        let (Some(bin), Some(rows), Some(first), Some(second), Some(widgets)) = (
            sizing_bin.upgrade(),
            rows.upgrade(),
            first.upgrade(),
            second.upgrade(),
            widgets
                .iter()
                .map(|widget| widget.upgrade())
                .collect::<Option<Vec<_>>>(),
        ) else {
            return;
        };
        let focus = rows
            .root()
            .and_then(|root| root.focus())
            .filter(|focus| focus.is_ancestor(&rows));
        for widget in &widgets {
            widget
                .parent()
                .and_downcast::<gtk::Box>()
                .expect("transport controls belong to native rows")
                .remove(widget);
        }
        let time = widgets[2]
            .downcast_ref::<gtk::Label>()
            .expect("transport time label");
        time.set_width_chars(if stacked { 0 } else { TIME_COLUMNS });
        time.set_max_width_chars(TIME_COLUMNS);
        time.set_ellipsize(if stacked {
            gtk::pango::EllipsizeMode::End
        } else {
            gtk::pango::EllipsizeMode::None
        });
        time.set_hexpand(stacked);
        time.set_halign(if stacked {
            gtk::Align::End
        } else {
            gtk::Align::Start
        });
        if stacked {
            first.append(&widgets[1]);
            first.append(&widgets[3]);
            second.append(&widgets[0]);
            second.append(&widgets[2]);
        } else {
            for widget in &widgets {
                first.append(widget);
            }
        }
        second.set_visible(stacked);
        bin.set_height_request(rows.measure(gtk::Orientation::Vertical, -1).0);
        if let Some(focus) = focus {
            focus.grab_focus();
        }
    });
    arrange(true);
    bin.set_width_request(
        bin.child()
            .expect("transport rows")
            .measure(gtk::Orientation::Horizontal, -1)
            .0,
    );
    arrange(false);
    let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        wide_minimum.next_down(),
        adw::LengthUnit::Px,
    ));
    let apply = arrange.clone();
    breakpoint.connect_apply(move |_| apply(true));
    breakpoint.connect_unapply(move |_| arrange(false));
    bin.add_breakpoint(breakpoint);
    let host = gtk::Box::new(gtk::Orientation::Vertical, 0);
    host.set_margin_top(GAP);
    host.set_margin_bottom(GAP);
    host.set_margin_start(GAP);
    host.set_margin_end(GAP);
    host.append(&bin);
    host
}
