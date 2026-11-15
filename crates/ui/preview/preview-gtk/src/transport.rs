use std::rc::Rc;

use adw::prelude::*;
use shrimply_components_gtk::layout::TRANSPORT_ROW_MIN_WIDTH;

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
