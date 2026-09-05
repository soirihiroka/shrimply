use core::pin::Pin;
use cxx_qt_lib::{QString, QStringList};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QStringList, modifier_values, cxx_name = "modifierValues")]
        #[qproperty(QStringList, modifier_labels, cxx_name = "modifierLabels")]
        #[qproperty(QString, expression_source, cxx_name = "expressionSource")]
        type DemoLogic = super::DemoLogicRust;

        #[qinvokable]
        #[cxx_name = "expressionOutput"]
        fn expression_output(self: &DemoLogic, source: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "graphPointTimes"]
        fn graph_point_times(self: &DemoLogic) -> QStringList;
        #[qinvokable]
        #[cxx_name = "graphPointValues"]
        fn graph_point_values(self: &DemoLogic, value: f64) -> QStringList;
        #[qinvokable]
        #[cxx_name = "graphSegments"]
        fn graph_segments(self: &DemoLogic, value: f64) -> QStringList;
    }

    impl cxx_qt::Initialize for DemoLogic {}
}

#[derive(Default)]
pub struct DemoLogicRust {
    modifier_values: QStringList,
    modifier_labels: QStringList,
    expression_source: QString,
}

impl cxx_qt::Initialize for qobject::DemoLogic {
    fn initialize(mut self: Pin<&mut Self>) {
        let names = shrimply_components_demo_core::modifier_names();
        self.as_mut().set_modifier_values(
            names
                .iter()
                .map(|name| QString::from(*name))
                .collect::<QStringList>(),
        );
        self.as_mut().set_modifier_labels(
            names
                .into_iter()
                .map(QString::from)
                .collect::<QStringList>(),
        );
        self.as_mut().set_expression_source(QString::from(
            shrimply_components_demo_core::EXPRESSION_SOURCE,
        ));
    }
}

impl qobject::DemoLogic {
    pub fn expression_output(&self, source: &QString) -> QString {
        QString::from(shrimply_components_demo_core::expression_output(
            &source.to_string(),
        ))
    }

    pub fn graph_point_times(&self) -> QStringList {
        raw_graph(0.0)
            .0
            .into_iter()
            .map(|point| exact_time(point.time))
            .collect()
    }

    pub fn graph_point_values(&self, value: f64) -> QStringList {
        raw_graph(value)
            .0
            .into_iter()
            .map(|point| QString::from(point.value.to_string()))
            .collect()
    }

    pub fn graph_segments(&self, value: f64) -> QStringList {
        raw_graph(value)
            .1
            .into_iter()
            .map(|segment| {
                QString::from(format!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    segment.owner_id,
                    exact_time(segment.start),
                    exact_time(segment.end),
                    segment.start_value,
                    segment.end_value,
                    segment.interpolation.label(),
                ))
            })
            .collect()
    }
}

fn raw_graph(
    value: f64,
) -> (
    Vec<shrimply_keyframe_graph_core::KeyframePoint>,
    Vec<shrimply_keyframe_graph_core::RawSegment>,
) {
    let shrimply_keyframe_graph_core::KeyframeGraph::RawValue {
        points, segments, ..
    } = shrimply_components_demo_core::property_graph(value)
    else {
        panic!("demo property graph is not a raw graph")
    };
    (points, segments)
}

fn exact_time(time: shrimply_math_core::Time) -> QString {
    QString::from(format!(
        "{}/{}",
        shrimply_math_core::fraction_numerator(time.seconds),
        shrimply_math_core::fraction_denominator(time.seconds),
    ))
}
