use core::ffi::c_void;
use core::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QPoint, QString, QStringList};
use shrimply_interpolation::Interpolation;
use shrimply_keyframe_graph_core::{
    FrameGraphAction, FrameGraphComponentAction, FrameGraphComponents, FrameGraphKey,
    FrameGraphKeyMove, FrameGraphModifiers, FrameGraphPointerButton, FrameGraphPointerPosition,
    FrameGraphScrollInput, FrameGraphState, KeyframeGraph, KeyframePoint, RawSegment, SpeedSegment,
};
use shrimply_math_color::Color;
use shrimply_math_core::{Time, fraction_denominator, fraction_numerator};
use shrimply_skia_adw_core::canvas::UVec2;
use shrimply_skia_gl::TimelineRenderer;
use uuid::Uuid;

type SharedGraph = Arc<Mutex<GraphModel>>;

const QT_WHEEL_ANGLE_UNITS_PER_STEP: f64 = 120.0;

struct GraphModel {
    state: FrameGraphComponents,
    context_owner: Option<Uuid>,
}

struct QtFrameGraphRenderer {
    graph: SharedGraph,
    renderer: TimelineRenderer,
}

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++Qt" {
        include!("frame_graph.h");
        #[qobject]
        #[namespace = "shrimply"]
        type FrameGraphItemBase;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qpoint.h");
        type QPoint = cxx_qt_lib::QPoint;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[base = FrameGraphItemBase]
        #[qproperty(bool, can_previous, cxx_name = "canPrevious")]
        #[qproperty(bool, can_next, cxx_name = "canNext")]
        #[qproperty(bool, key_at_playhead, cxx_name = "keyAtPlayhead")]
        #[qproperty(f64, graph_value, cxx_name = "graphValue")]
        #[qproperty(i32, preferred_height, cxx_name = "preferredHeight")]
        #[qproperty(i32, interpolation_count, cxx_name = "interpolationCount")]
        type FrameGraphItem = super::FrameGraphItemRust;

        #[inherit]
        #[cxx_name = "update"]
        fn request_update(self: Pin<&mut FrameGraphItem>);
        #[inherit]
        #[cxx_name = "width"]
        fn item_width(self: &FrameGraphItem) -> f64;
        #[inherit]
        #[cxx_name = "height"]
        fn item_height(self: &FrameGraphItem) -> f64;

        #[cxx_override]
        #[cxx_name = "frameGraphHandle"]
        fn frame_graph_handle(self: &FrameGraphItem) -> usize;

        #[qinvokable]
        #[cxx_name = "pointerMoved"]
        fn pointer_moved(self: Pin<&mut FrameGraphItem>, x: f64, y: f64);
        #[qinvokable]
        #[cxx_name = "pointerLeft"]
        fn pointer_left(self: Pin<&mut FrameGraphItem>);
        #[qinvokable]
        fn begin(
            self: Pin<&mut FrameGraphItem>,
            button: i32,
            x: f64,
            y: f64,
            control: bool,
            shift: bool,
        ) -> i32;
        #[qinvokable]
        #[cxx_name = "updatePointer"]
        fn update_pointer(self: Pin<&mut FrameGraphItem>, x: f64, y: f64);
        #[qinvokable]
        #[cxx_name = "endPointer"]
        fn end_pointer(self: Pin<&mut FrameGraphItem>);
        #[qinvokable]
        fn scroll(
            self: Pin<&mut FrameGraphItem>,
            pixel_delta: &QPoint,
            angle_delta: &QPoint,
            x: f64,
            y: f64,
            control: bool,
        ) -> bool;
        #[qinvokable]
        #[cxx_name = "handleKey"]
        fn handle_key(self: Pin<&mut FrameGraphItem>, key: i32);
        #[qinvokable]
        #[cxx_name = "previousKey"]
        fn previous_key(self: Pin<&mut FrameGraphItem>);
        #[qinvokable]
        #[cxx_name = "toggleKey"]
        fn toggle_key(self: Pin<&mut FrameGraphItem>);
        #[qinvokable]
        #[cxx_name = "nextKey"]
        fn next_key(self: Pin<&mut FrameGraphItem>);
        #[qinvokable]
        #[cxx_name = "editGraphValue"]
        fn edit_graph_value(self: Pin<&mut FrameGraphItem>, value: f64);
        #[qinvokable]
        #[cxx_name = "configureGraphCurrentValue"]
        fn configure_graph_current_value(self: Pin<&mut FrameGraphItem>, value: f64);
        #[qinvokable]
        #[cxx_name = "editGraphComponentValue"]
        fn edit_graph_component_value(self: Pin<&mut FrameGraphItem>, component: i32, value: f64);
        #[qinvokable]
        #[cxx_name = "editGraphPair"]
        fn edit_graph_pair(
            self: Pin<&mut FrameGraphItem>,
            first: f64,
            second: f64,
            active_component: i32,
            first_changed: bool,
            second_changed: bool,
        );
        #[qinvokable]
        #[cxx_name = "configureGraphValue"]
        fn configure_graph_value(self: Pin<&mut FrameGraphItem>, value: f64);
        #[qinvokable]
        #[cxx_name = "configureGraphPair"]
        fn configure_graph_pair(
            self: Pin<&mut FrameGraphItem>,
            first: f64,
            second: f64,
            active_component: i32,
        );
        #[qinvokable]
        #[cxx_name = "replaceStepGraph"]
        fn replace_step_graph(
            self: Pin<&mut FrameGraphItem>,
            component: i32,
            times: &QStringList,
            values: &QStringList,
        );
        #[qinvokable]
        #[cxx_name = "reconcileStepGraphMoves"]
        fn reconcile_step_graph_moves(
            self: Pin<&mut FrameGraphItem>,
            component: i32,
            old_times: &QStringList,
            raw_times: &QStringList,
            times: &QStringList,
        );
        #[qinvokable]
        #[cxx_name = "rollbackStepGraphMoves"]
        fn rollback_step_graph_moves(
            self: Pin<&mut FrameGraphItem>,
            component: i32,
            old_times: &QStringList,
            raw_times: &QStringList,
        );
        #[qinvokable]
        #[cxx_name = "replaceRawGraph"]
        fn replace_raw_graph(
            self: Pin<&mut FrameGraphItem>,
            component: i32,
            point_times: &QStringList,
            point_values: &QStringList,
            segments: &QStringList,
            static_value: f64,
        );
        #[qinvokable]
        #[cxx_name = "replaceSpeedGraph"]
        fn replace_speed_graph(
            self: Pin<&mut FrameGraphItem>,
            component: i32,
            keys: &QStringList,
            segments: &QStringList,
            static_value: f64,
        );
        #[qinvokable]
        #[cxx_name = "setGraphRange"]
        fn set_graph_range(
            self: Pin<&mut FrameGraphItem>,
            start_numerator: i64,
            start_denominator: i64,
            end_numerator: i64,
            end_denominator: i64,
        );
        #[qinvokable]
        #[cxx_name = "setGraphFrameStep"]
        fn set_graph_frame_step(self: Pin<&mut FrameGraphItem>, numerator: i64, denominator: i64);
        #[qinvokable]
        #[cxx_name = "setGraphPlayhead"]
        fn set_graph_playhead(self: Pin<&mut FrameGraphItem>, numerator: i64, denominator: i64);
        #[qinvokable]
        #[cxx_name = "setGraphSnapping"]
        fn set_graph_snapping(self: Pin<&mut FrameGraphItem>, enabled: bool, radius_px: f64);
        #[qinvokable]
        #[cxx_name = "setGraphExternalClipboard"]
        fn set_graph_external_clipboard(self: Pin<&mut FrameGraphItem>, enabled: bool);
        #[qinvokable]
        #[cxx_name = "setGraphTextInterpolation"]
        fn set_graph_text_interpolation(self: Pin<&mut FrameGraphItem>, enabled: bool);
        #[qinvokable]
        #[cxx_name = "activateGraphComponent"]
        fn activate_graph_component(self: Pin<&mut FrameGraphItem>, component: i32);
        #[qinvokable]
        #[cxx_name = "setInterpolation"]
        fn set_interpolation(self: Pin<&mut FrameGraphItem>, index: i32) -> bool;
        #[qinvokable]
        #[cxx_name = "setTextInterpolation"]
        fn set_text_interpolation(self: Pin<&mut FrameGraphItem>, index: i32) -> bool;
        #[qinvokable]
        #[cxx_name = "interpolationLabel"]
        fn interpolation_label(self: &FrameGraphItem, index: i32) -> QString;

        #[qsignal]
        #[cxx_name = "togglePlayback"]
        fn toggle_playback(self: Pin<&mut FrameGraphItem>);
        #[qsignal]
        #[cxx_name = "editFinished"]
        fn edit_finished(self: Pin<&mut FrameGraphItem>);
        #[qsignal]
        #[cxx_name = "playheadChanged"]
        fn playhead_changed(
            self: Pin<&mut FrameGraphItem>,
            component: i32,
            numerator: i64,
            denominator: i64,
        );
        #[qsignal]
        #[cxx_name = "keysChanged"]
        fn keys_changed(
            self: Pin<&mut FrameGraphItem>,
            component: i32,
            times: QStringList,
            values: QStringList,
        );
        #[qsignal]
        #[cxx_name = "keysMoved"]
        fn keys_moved(
            self: Pin<&mut FrameGraphItem>,
            component: i32,
            old_times: QStringList,
            times: QStringList,
            values: QStringList,
        );
        #[qsignal]
        #[cxx_name = "keysDeleted"]
        fn keys_deleted(self: Pin<&mut FrameGraphItem>, component: i32, times: QStringList);
        #[qsignal]
        #[cxx_name = "keyAdded"]
        fn key_added(
            self: Pin<&mut FrameGraphItem>,
            component: i32,
            numerator: i64,
            denominator: i64,
            value: f64,
        );
        #[qsignal]
        #[cxx_name = "keysPasted"]
        fn keys_pasted(
            self: Pin<&mut FrameGraphItem>,
            component: i32,
            times: QStringList,
            values: QStringList,
        );
        #[qsignal]
        #[cxx_name = "copyRequested"]
        fn copy_requested(self: Pin<&mut FrameGraphItem>, component: i32, times: QStringList);
        #[qsignal]
        #[cxx_name = "pasteRequested"]
        fn paste_requested(
            self: Pin<&mut FrameGraphItem>,
            component: i32,
            numerator: i64,
            denominator: i64,
        );
        #[qsignal]
        #[cxx_name = "textInterpolationRequested"]
        fn text_interpolation_requested(
            self: Pin<&mut FrameGraphItem>,
            component: i32,
            owner_id: QString,
            x: f64,
            y: f64,
        );
        #[qsignal]
        #[cxx_name = "textInterpolationChanged"]
        fn text_interpolation_changed(
            self: Pin<&mut FrameGraphItem>,
            component: i32,
            owner_id: QString,
            index: i32,
        );
        #[qsignal]
        #[cxx_name = "interpolationRequested"]
        fn interpolation_requested(
            self: Pin<&mut FrameGraphItem>,
            component: i32,
            owner_id: QString,
            index: i32,
            x: f64,
            y: f64,
        );
        #[qsignal]
        #[cxx_name = "interpolationChanged"]
        fn interpolation_changed(
            self: Pin<&mut FrameGraphItem>,
            component: i32,
            owner_id: QString,
            index: i32,
        );
    }

    impl cxx_qt::Constructor<()> for FrameGraphItem {}
}

pub struct FrameGraphItemRust {
    can_previous: bool,
    can_next: bool,
    key_at_playhead: bool,
    graph_value: f64,
    preferred_height: i32,
    interpolation_count: i32,
    graph: SharedGraph,
}

impl Default for FrameGraphItemRust {
    fn default() -> Self {
        let state = FrameGraphState::constant(0.0);
        let status = state.status();
        Self {
            can_previous: status.can_previous,
            can_next: status.can_next,
            key_at_playhead: status.key_at_playhead,
            graph_value: status.value,
            preferred_height: state.preferred_height(),
            interpolation_count: Interpolation::KEYFRAME.len() as i32,
            graph: Arc::new(Mutex::new(GraphModel {
                state: FrameGraphComponents::single(state),
                context_owner: None,
            })),
        }
    }
}

impl cxx_qt::Initialize for qobject::FrameGraphItem {
    fn initialize(self: Pin<&mut Self>) {}
}

impl qobject::FrameGraphItem {
    pub fn frame_graph_handle(&self) -> usize {
        Arc::as_ptr(&self.rust().graph) as usize
    }

    pub fn pointer_moved(mut self: Pin<&mut Self>, x: f64, y: f64) {
        self.rust().lock().state.pointer_moved(x, y);
        self.as_mut().request_update();
    }

    pub fn pointer_left(mut self: Pin<&mut Self>) {
        self.rust().lock().state.pointer_left();
        self.as_mut().request_update();
    }

    pub fn begin(
        mut self: Pin<&mut Self>,
        button: i32,
        x: f64,
        y: f64,
        control: bool,
        shift: bool,
    ) -> i32 {
        let button = match button {
            0 => FrameGraphPointerButton::Primary,
            1 => FrameGraphPointerButton::Middle,
            2 => FrameGraphPointerButton::Secondary,
            _ => panic!("Qt passed an invalid frame graph pointer button: {button}"),
        };
        let width = self.item_width();
        let height = self.item_height();
        let (actions, selected) = {
            let mut model = self.rust().lock();
            model.context_owner = None;
            let actions = model.state.active_actions(|state| {
                state.begin_pointer(
                    button,
                    x,
                    y,
                    width,
                    height,
                    FrameGraphModifiers { control, shift },
                )
            });
            let selected =
                actions
                    .iter()
                    .find_map(|component_action| match &component_action.action {
                        FrameGraphAction::InterpolationRequested {
                            owner_id,
                            interpolation,
                            ..
                        } => Some((*owner_id, Some(*interpolation))),
                        FrameGraphAction::TextInterpolationRequested { owner_id, .. } => {
                            Some((*owner_id, None))
                        }
                        _ => None,
                    });
            if let Some((owner_id, _)) = selected {
                model.context_owner = Some(owner_id);
            }
            (
                actions,
                selected.and_then(|(_, interpolation)| interpolation),
            )
        };
        self.as_mut().finish(actions);
        selected.map_or(-1, interpolation_index)
    }

    pub fn update_pointer(mut self: Pin<&mut Self>, x: f64, y: f64) {
        let width = self.item_width();
        let height = self.item_height();
        let actions = self
            .rust()
            .lock()
            .state
            .active_actions(|state| state.update_pointer(x, y, width, height));
        self.as_mut().finish(actions);
    }

    pub fn end_pointer(mut self: Pin<&mut Self>) {
        let actions = self
            .rust()
            .lock()
            .state
            .active_actions(FrameGraphState::end_pointer);
        self.as_mut().finish(actions);
        self.as_mut().request_update();
    }

    pub fn scroll(
        mut self: Pin<&mut Self>,
        pixel_delta: &QPoint,
        angle_delta: &QPoint,
        x: f64,
        y: f64,
        control: bool,
    ) -> bool {
        // QWheelEvent vertical deltas point up while GDK scroll controllers
        // use positive Y for scrolling down. Preserve native pixel precision;
        // angle deltas use Qt's documented 120 units per wheel detent.
        let has_pixel_delta = !pixel_delta.is_null();
        let (dx, dy, input) = if has_pixel_delta {
            (
                -f64::from(pixel_delta.x()),
                -f64::from(pixel_delta.y()),
                FrameGraphScrollInput::Surface,
            )
        } else {
            (
                -f64::from(angle_delta.x()) / QT_WHEEL_ANGLE_UNITS_PER_STEP,
                -f64::from(angle_delta.y()) / QT_WHEEL_ANGLE_UNITS_PER_STEP,
                FrameGraphScrollInput::Wheel,
            )
        };
        let width = self.item_width();
        let height = self.item_height();
        let handled = self.rust().lock().state.scroll(
            dx,
            dy,
            FrameGraphPointerPosition {
                x,
                y,
                width,
                height,
            },
            control,
            input,
        );
        if handled {
            self.as_mut().request_update();
        }
        handled
    }

    pub fn handle_key(mut self: Pin<&mut Self>, key: i32) {
        let key = match key {
            0 => FrameGraphKey::PreviousFrame,
            1 => FrameGraphKey::NextFrame,
            2 => FrameGraphKey::Start,
            3 => FrameGraphKey::End,
            4 => FrameGraphKey::ZoomIn,
            5 => FrameGraphKey::ZoomOut,
            6 => FrameGraphKey::Delete,
            7 => FrameGraphKey::Copy,
            8 => FrameGraphKey::Paste,
            9 => FrameGraphKey::TogglePlayback,
            _ => panic!("Qt passed an invalid frame graph key: {key}"),
        };
        let actions = self
            .rust()
            .lock()
            .state
            .active_actions(|state| state.key(key));
        self.as_mut().finish(actions);
    }

    pub fn previous_key(mut self: Pin<&mut Self>) {
        let actions = self
            .rust()
            .lock()
            .state
            .active_actions(FrameGraphState::previous_key);
        self.as_mut().finish(actions);
    }

    pub fn toggle_key(mut self: Pin<&mut Self>) {
        let actions = self
            .rust()
            .lock()
            .state
            .active_actions(FrameGraphState::toggle_key);
        self.as_mut().finish(actions);
    }

    pub fn next_key(mut self: Pin<&mut Self>) {
        let actions = self
            .rust()
            .lock()
            .state
            .active_actions(FrameGraphState::next_key);
        self.as_mut().finish(actions);
    }

    pub fn edit_graph_value(mut self: Pin<&mut Self>, value: f64) {
        let actions = self
            .rust()
            .lock()
            .state
            .active_actions(|state| state.set_value(value));
        self.as_mut().finish(actions);
    }

    pub fn configure_graph_current_value(mut self: Pin<&mut Self>, value: f64) {
        self.rust().lock().state.set_value(value);
        self.as_mut().refresh_graph();
    }

    pub fn edit_graph_component_value(mut self: Pin<&mut Self>, component: i32, value: f64) {
        let component = usize::try_from(component).expect("non-negative frame graph component");
        let actions = {
            let mut model = self.rust().lock();
            model.context_owner = None;
            model
                .state
                .set_component_values(component, &[(component, value)])
        };
        self.as_mut().finish(actions);
    }

    pub fn edit_graph_pair(
        mut self: Pin<&mut Self>,
        first: f64,
        second: f64,
        active_component: i32,
        first_changed: bool,
        second_changed: bool,
    ) {
        let active_component =
            usize::try_from(active_component).expect("non-negative frame graph component");
        let values = [(0, first, first_changed), (1, second, second_changed)]
            .into_iter()
            .filter_map(|(component, value, changed)| changed.then_some((component, value)))
            .collect::<Vec<_>>();
        let actions = {
            let mut model = self.rust().lock();
            model.context_owner = None;
            model.state.set_component_values(active_component, &values)
        };
        self.as_mut().finish(actions);
    }

    pub fn configure_graph_value(mut self: Pin<&mut Self>, value: f64) {
        self.rust().lock().state = FrameGraphComponents::single(FrameGraphState::constant(value));
        self.as_mut().sync_status();
        self.as_mut().request_update();
    }

    pub fn configure_graph_pair(
        mut self: Pin<&mut Self>,
        first: f64,
        second: f64,
        active_component: i32,
    ) {
        let active_component =
            usize::try_from(active_component).expect("non-negative frame graph component");
        self.rust().lock().state =
            FrameGraphComponents::constant_values(&[first, second], active_component);
        self.as_mut().sync_status();
        self.as_mut().request_update();
    }

    pub fn replace_step_graph(
        self: Pin<&mut Self>,
        component: i32,
        times: &QStringList,
        values: &QStringList,
    ) {
        self.replace_component_graph(
            component,
            KeyframeGraph::Step {
                points: parse_points(times, values),
            },
        );
    }

    pub fn reconcile_step_graph_moves(
        self: Pin<&mut Self>,
        component: i32,
        old_times: &QStringList,
        raw_times: &QStringList,
        times: &QStringList,
    ) {
        assert_eq!(
            old_times.len(),
            raw_times.len(),
            "frame graph move columns differ"
        );
        assert_eq!(
            raw_times.len(),
            times.len(),
            "frame graph move columns differ"
        );
        let component = usize::try_from(component).expect("non-negative frame graph component");
        let moves = old_times
            .iter()
            .zip(raw_times.iter())
            .zip(times.iter())
            .map(|((old_time, raw_time), time)| {
                (parse_time(old_time), parse_time(raw_time), parse_time(time))
            })
            .collect::<Vec<_>>();
        self.update_graph(|graph| {
            graph
                .state
                .reconcile_component_step_moves(component, &moves);
        });
    }

    pub fn rollback_step_graph_moves(
        self: Pin<&mut Self>,
        component: i32,
        old_times: &QStringList,
        raw_times: &QStringList,
    ) {
        assert_eq!(
            old_times.len(),
            raw_times.len(),
            "frame graph move columns differ"
        );
        let component = usize::try_from(component).expect("non-negative frame graph component");
        let moves = old_times
            .iter()
            .zip(raw_times.iter())
            .map(|(old_time, raw_time)| (parse_time(old_time), parse_time(raw_time)))
            .collect::<Vec<_>>();
        self.update_graph(|graph| {
            graph.state.rollback_component_step_moves(component, &moves);
        });
    }

    pub fn replace_raw_graph(
        self: Pin<&mut Self>,
        component: i32,
        point_times: &QStringList,
        point_values: &QStringList,
        segments: &QStringList,
        static_value: f64,
    ) {
        assert!(
            static_value.is_finite(),
            "frame graph static value is not finite"
        );
        self.replace_component_graph(
            component,
            KeyframeGraph::RawValue {
                points: parse_points(point_times, point_values),
                segments: segments.iter().map(parse_raw_segment).collect(),
                static_value,
            },
        );
    }

    pub fn replace_speed_graph(
        self: Pin<&mut Self>,
        component: i32,
        keys: &QStringList,
        segments: &QStringList,
        static_value: f64,
    ) {
        assert!(
            static_value.is_finite(),
            "frame graph static value is not finite"
        );
        self.replace_component_graph(
            component,
            KeyframeGraph::Speed {
                segments: segments.iter().map(parse_speed_segment).collect(),
                keys: keys.iter().map(parse_time).collect(),
                static_value,
            },
        );
    }

    pub fn set_graph_range(
        self: Pin<&mut Self>,
        start_numerator: i64,
        start_denominator: i64,
        end_numerator: i64,
        end_denominator: i64,
    ) {
        let start = exact_time(start_numerator, start_denominator);
        let end = exact_time(end_numerator, end_denominator);
        self.update_graph(|graph| graph.state.set_item_range((start, end)));
    }

    pub fn set_graph_frame_step(self: Pin<&mut Self>, numerator: i64, denominator: i64) {
        let frame_step = exact_time(numerator, denominator);
        self.update_graph(|graph| graph.state.set_frame_step(frame_step));
    }

    pub fn set_graph_playhead(self: Pin<&mut Self>, numerator: i64, denominator: i64) {
        let playhead = exact_time(numerator, denominator);
        self.update_graph(|graph| graph.state.set_playhead(playhead));
    }

    pub fn set_graph_snapping(self: Pin<&mut Self>, enabled: bool, radius_px: f64) {
        self.update_graph(|graph| graph.state.set_snapping(enabled, radius_px));
    }

    pub fn set_graph_external_clipboard(self: Pin<&mut Self>, enabled: bool) {
        self.update_graph(|graph| graph.state.set_external_clipboard(enabled));
    }

    pub fn set_graph_text_interpolation(self: Pin<&mut Self>, enabled: bool) {
        self.update_graph(|graph| graph.state.set_text_interpolation(enabled));
    }

    pub fn activate_graph_component(mut self: Pin<&mut Self>, component: i32) {
        let component = usize::try_from(component).expect("non-negative frame graph component");
        let mut model = self.rust().lock();
        model.context_owner = None;
        model.state.activate(component);
        drop(model);
        self.as_mut().sync_status();
        self.as_mut().request_update();
    }

    pub fn set_interpolation(mut self: Pin<&mut Self>, index: i32) -> bool {
        let Some(interpolation) = usize::try_from(index)
            .ok()
            .and_then(|index| Interpolation::KEYFRAME.get(index))
            .copied()
        else {
            return false;
        };
        let owner_id = {
            let mut model = self.rust().lock();
            let Some(owner_id) = model.context_owner.take() else {
                return false;
            };
            model.state.set_interpolation(owner_id, interpolation);
            (model.state.active_component(), owner_id)
        };
        self.as_mut().interpolation_changed(
            component_index(owner_id.0),
            QString::from(owner_id.1.to_string()),
            index,
        );
        self.as_mut().request_update();
        true
    }

    pub fn set_text_interpolation(mut self: Pin<&mut Self>, index: i32) -> bool {
        if index < 0 {
            return false;
        }
        let owner_id = {
            let mut model = self.rust().lock();
            let Some(owner_id) = model.context_owner.take() else {
                return false;
            };
            (model.state.active_component(), owner_id)
        };
        self.as_mut().text_interpolation_changed(
            component_index(owner_id.0),
            QString::from(owner_id.1.to_string()),
            index,
        );
        true
    }

    pub fn interpolation_label(&self, index: i32) -> QString {
        usize::try_from(index)
            .ok()
            .and_then(|index| Interpolation::KEYFRAME.get(index))
            .map_or_else(QString::default, |interpolation| {
                QString::from(interpolation.label())
            })
    }

    fn replace_component_graph(mut self: Pin<&mut Self>, component: i32, graph: KeyframeGraph) {
        let component = usize::try_from(component).expect("non-negative frame graph component");
        self.rust()
            .lock()
            .state
            .replace_component_graph(component, graph);
        self.as_mut().refresh_graph();
    }

    fn update_graph(mut self: Pin<&mut Self>, update: impl FnOnce(&mut GraphModel)) {
        update(&mut self.rust().lock());
        self.as_mut().refresh_graph();
    }

    fn refresh_graph(mut self: Pin<&mut Self>) {
        self.as_mut().sync_status();
        self.as_mut().request_update();
    }

    fn finish(mut self: Pin<&mut Self>, actions: Vec<FrameGraphComponentAction>) {
        for FrameGraphComponentAction { component, action } in actions {
            let component = component_index(component);
            match action {
                FrameGraphAction::PlayheadChanged(time) => {
                    let (numerator, denominator) = time_parts(time);
                    self.as_mut()
                        .playhead_changed(component, numerator, denominator);
                }
                FrameGraphAction::KeysChanged(points) => {
                    let (times, values) = point_lists(&points);
                    self.as_mut().keys_changed(component, times, values);
                }
                FrameGraphAction::KeysMoved(moves) => {
                    let (old_times, times, values) = move_lists(&moves);
                    self.as_mut()
                        .keys_moved(component, old_times, times, values);
                }
                FrameGraphAction::KeysDeleted(times) => {
                    self.as_mut().keys_deleted(component, time_list(&times));
                }
                FrameGraphAction::KeyAdded(point) => {
                    let (numerator, denominator) = time_parts(point.time);
                    self.as_mut()
                        .key_added(component, numerator, denominator, point.value);
                }
                FrameGraphAction::KeysPasted(points) => {
                    let (times, values) = point_lists(&points);
                    self.as_mut().keys_pasted(component, times, values);
                }
                FrameGraphAction::CopyRequested(times) => {
                    self.as_mut().copy_requested(component, time_list(&times));
                }
                FrameGraphAction::PasteRequested(time) => {
                    let (numerator, denominator) = time_parts(time);
                    self.as_mut()
                        .paste_requested(component, numerator, denominator);
                }
                FrameGraphAction::TogglePlayback => self.as_mut().toggle_playback(),
                FrameGraphAction::EditFinished => self.as_mut().edit_finished(),
                FrameGraphAction::InterpolationRequested {
                    owner_id,
                    interpolation,
                    x,
                    y,
                } => {
                    self.as_mut().interpolation_requested(
                        component,
                        QString::from(owner_id.to_string()),
                        interpolation_index(interpolation),
                        x,
                        y,
                    );
                }
                FrameGraphAction::TextInterpolationRequested { owner_id, x, y } => {
                    self.as_mut().text_interpolation_requested(
                        component,
                        QString::from(owner_id.to_string()),
                        x,
                        y,
                    );
                }
            }
        }
        self.as_mut().sync_status();
        self.as_mut().request_update();
    }

    fn sync_status(mut self: Pin<&mut Self>) {
        let (status, preferred_height) = {
            let graph = self.rust().lock();
            (graph.state.status(), graph.state.preferred_height())
        };
        self.as_mut().set_can_previous(status.can_previous);
        self.as_mut().set_can_next(status.can_next);
        self.as_mut().set_key_at_playhead(status.key_at_playhead);
        self.as_mut().set_graph_value(status.value);
        self.as_mut().set_preferred_height(preferred_height);
    }
}

impl FrameGraphItemRust {
    fn lock(&self) -> MutexGuard<'_, GraphModel> {
        self.graph
            .lock()
            .unwrap_or_else(|_| panic!("Qt frame graph state lock was poisoned"))
    }
}

fn component_index(component: usize) -> i32 {
    i32::try_from(component).expect("frame graph component index exceeds Qt's range")
}

fn interpolation_index(interpolation: Interpolation) -> i32 {
    Interpolation::KEYFRAME
        .iter()
        .position(|candidate| *candidate == interpolation)
        .and_then(|index| i32::try_from(index).ok())
        .expect("frame graph interpolation is not available in Qt")
}

fn parse_points(times: &QStringList, values: &QStringList) -> Vec<KeyframePoint> {
    assert_eq!(
        times.len(),
        values.len(),
        "frame graph point columns have different lengths"
    );
    times
        .iter()
        .zip(values.iter())
        .map(|(time, value)| KeyframePoint {
            time: parse_time(time),
            value: parse_value(value),
        })
        .collect()
}

fn parse_time(value: &QString) -> Time {
    let value = value.to_string();
    parse_time_text(&value)
}

fn parse_time_text(value: &str) -> Time {
    let (numerator, denominator) = value
        .split_once('/')
        .unwrap_or_else(|| panic!("frame graph time is not an exact fraction: {value}"));
    exact_time(
        numerator
            .parse()
            .unwrap_or_else(|_| panic!("frame graph time has an invalid numerator: {value}")),
        denominator
            .parse()
            .unwrap_or_else(|_| panic!("frame graph time has an invalid denominator: {value}")),
    )
}

fn exact_time(numerator: i64, denominator: i64) -> Time {
    assert_ne!(denominator, 0, "frame graph time denominator is zero");
    Time::from_fraction(numerator, denominator)
}

fn parse_value(value: &QString) -> f64 {
    let text = value.to_string();
    parse_value_text(&text)
}

fn parse_value_text(text: &str) -> f64 {
    let value = text
        .parse::<f64>()
        .unwrap_or_else(|_| panic!("frame graph value is invalid: {text}"));
    assert!(value.is_finite(), "frame graph value is not finite: {text}");
    value
}

fn parse_uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).unwrap_or_else(|_| panic!("frame graph UUID is invalid: {value}"))
}

fn parse_interpolation(value: &str) -> Interpolation {
    let interpolation = value
        .parse::<usize>()
        .ok()
        .and_then(|index| Interpolation::KEYFRAME.get(index))
        .copied()
        .or_else(|| {
            Interpolation::KEYFRAME
                .into_iter()
                .find(|interpolation| interpolation.label() == value)
        });
    interpolation.unwrap_or_else(|| panic!("frame graph interpolation is invalid: {value}"))
}

fn parse_raw_segment(value: &QString) -> RawSegment {
    let value = value.to_string();
    let fields = segment_fields(&value, 6, "raw");
    RawSegment {
        owner_id: parse_uuid(fields[0]),
        start: parse_time_text(fields[1]),
        end: parse_time_text(fields[2]),
        start_value: parse_value_text(fields[3]),
        end_value: parse_value_text(fields[4]),
        interpolation: parse_interpolation(fields[5]),
    }
}

fn parse_speed_segment(value: &QString) -> SpeedSegment {
    let value = value.to_string();
    let fields = segment_fields(&value, 5, "speed");
    SpeedSegment {
        owner_id: parse_uuid(fields[0]),
        start: parse_time_text(fields[1]),
        end: parse_time_text(fields[2]),
        value: parse_value_text(fields[3]),
        interpolation: parse_interpolation(fields[4]),
    }
}

fn segment_fields<'a>(value: &'a str, count: usize, kind: &str) -> Vec<&'a str> {
    let fields = value.split('\t').collect::<Vec<_>>();
    assert_eq!(
        fields.len(),
        count,
        "{kind} frame graph segment has the wrong number of fields"
    );
    fields
}

fn time_parts(time: Time) -> (i64, i64) {
    (
        fraction_numerator(time.seconds),
        fraction_denominator(time.seconds),
    )
}

fn time_list(times: &[Time]) -> QStringList {
    times.iter().copied().map(time_text).collect()
}

fn time_text(time: Time) -> QString {
    let (numerator, denominator) = time_parts(time);
    QString::from(format!("{numerator}/{denominator}"))
}

fn point_lists(points: &[KeyframePoint]) -> (QStringList, QStringList) {
    (
        points.iter().map(|point| time_text(point.time)).collect(),
        points
            .iter()
            .map(|point| QString::from(point.value.to_string()))
            .collect(),
    )
}

fn move_lists(moves: &[FrameGraphKeyMove]) -> (QStringList, QStringList, QStringList) {
    (
        moves
            .iter()
            .map(|key_move| time_text(key_move.old_time))
            .collect(),
        moves
            .iter()
            .map(|key_move| time_text(key_move.time))
            .collect(),
        moves
            .iter()
            .map(|key_move| QString::from(key_move.value.to_string()))
            .collect(),
    )
}

fn graph_from_raw(graph: *const c_void) -> SharedGraph {
    assert!(!graph.is_null(), "Qt passed a null frame graph");
    let graph = graph.cast::<Mutex<GraphModel>>();
    unsafe { Arc::increment_strong_count(graph) };
    unsafe { Arc::from_raw(graph) }
}

#[unsafe(no_mangle)]
extern "C" fn shrimply_qt_frame_graph_renderer_new(graph: *const c_void) -> *mut c_void {
    Box::into_raw(Box::new(QtFrameGraphRenderer {
        graph: graph_from_raw(graph),
        renderer: TimelineRenderer::new(),
    }))
    .cast()
}

#[unsafe(no_mangle)]
unsafe extern "C" fn shrimply_qt_frame_graph_renderer_free(renderer: *mut c_void) {
    assert!(!renderer.is_null(), "Qt passed a null frame graph renderer");
    drop(unsafe { Box::from_raw(renderer.cast::<QtFrameGraphRenderer>()) });
}

#[unsafe(no_mangle)]
extern "C" fn shrimply_qt_frame_graph_render(
    renderer: *mut c_void,
    width: u32,
    height: u32,
    scale: f32,
    red: f32,
    green: f32,
    blue: f32,
    alpha: f32,
    dark: bool,
) -> i32 {
    assert!(!renderer.is_null(), "Qt passed a null frame graph renderer");
    shrimply_cross_ui_theme::set_dark(dark);
    let renderer = unsafe { &mut *renderer.cast::<QtFrameGraphRenderer>() };
    let result = (|| {
        let painter = renderer.renderer.begin_frame(
            UVec2::new(width, height),
            scale,
            Color::new(red, green, blue, alpha),
        )?;
        let animating = {
            let mut graph = renderer
                .graph
                .lock()
                .unwrap_or_else(|_| panic!("Qt frame graph state lock was poisoned"));
            graph.state.draw(
                &painter,
                f64::from(width) / f64::from(scale),
                f64::from(height) / f64::from(scale),
            );
            graph.state.is_animating()
        };
        renderer.renderer.end_frame()?;
        Ok::<bool, String>(animating)
    })();
    match result {
        Ok(animating) => i32::from(animating),
        Err(error) => {
            eprintln!("Qt frame graph OpenGL render failed: {error}");
            -1
        }
    }
}
