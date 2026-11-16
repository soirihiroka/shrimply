use sanim_core::{Fraction, Geometry, Unit};
use sanim_lang::{Engine, MemorySources};

const SCALES: &str = r#"
let xs=LinearScale(domain=[-4, 4],range=[-200pt, 200pt])
let ys=LinearScale(domain=[0, 1],range=[120pt, -120pt])
"#;
fn prepare(source: &str) -> sanim_lang::Prepared {
    let mut files = MemorySources::default();
    files.insert("main.sanim", format!("{SCALES}\n{source}\nAnimation()"));
    Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_or_else(|error| panic!("{error}"))
}

#[test]
fn curves_distributions_areas_and_legends_are_native_typed_shapes() {
    let prepared = prepare(
        r#"
let data=[{x=-3,y=0.2},{x=-1,y=0.4},{x=1,y=0.1},{x=3,y=0.7}]
let curve=LinePlot(data=data,x_scale=xs,y_scale=ys,interpolation="monotone")
let area=AreaPlot(data=data,x_scale=xs,y_scale=ys)
let band=BandPlot(data=[{x=-3,low=0.1,high=0.3},{x=3,low=0.2,high=0.6}],x_scale=xs,y_scale=ys)
let histogram=Histogram(values=[-3,-2,-1,0,1,2,3,3],bins=8,normalization="probability",x_scale=xs,y_scale=ys)
let density=DensityPlot(values=[-2,-1,0,1,2],bandwidth=0.6,x_scale=xs,y_scale=ys,points=64)
let normal=NormalCurve(x_scale=xs,y_scale=ys)
let ecdf=ECDFPlot(values=[-2,0,0,2],x_scale=xs,y_scale=ys)
let legend=Legend(labels=["Measured","Normal"],colors=[Color("blue3"),Color("orange3")])
Animation(children=[curve])
Animation(children=[area])
Animation(children=[band])
Animation(children=[histogram])
Animation(children=[density])
Animation(children=[normal])
Animation(children=[ecdf])
Animation(children=[legend])
wait(1s)
"#,
    );
    let Geometry::Polyline { points, .. } = prepared
        .animation
        .find("curve/curve")
        .unwrap()
        .geometry
        .as_ref()
    else {
        panic!("batched curve")
    };
    assert_eq!(points.len(), 25);
    assert!(
        matches!(prepared.animation.find("area/area").unwrap().geometry.as_ref(),Geometry::Polygon{points}if points.len()==6)
    );
    assert!(
        matches!(prepared.animation.find("density/curve").unwrap().geometry.as_ref(),Geometry::Polyline{points,..}if points.len()==64)
    );
    assert!(matches!(
        prepared
            .animation
            .find("histogram/bin/7")
            .unwrap()
            .geometry
            .as_ref(),
        Geometry::Box { .. }
    ));
    assert!(prepared.animation.find("legend/label/1").is_some());
    assert_eq!(prepared.animation.duration, Fraction::from(1));
}

#[test]
fn plot_and_scatter_share_exact_coordinates_despite_asymmetric_bounds() {
    let prepared = prepare(
        r#"
let data=[{x=-3,y=0.2},{x=1,y=0.4}]
let curve=LinePlot(data=data,x_scale=xs,y_scale=ys)
let markers=Scatter(data=data,x_scale=xs,y_scale=ys)
Animation(children=[curve])
Animation(children=[markers])
"#,
    );
    let elements = prepared
        .animation
        .layout(0.into(), 400.into(), 300.into())
        .unwrap();
    let Geometry::Box {
        children: curve, ..
    } = elements[0].geometry.as_ref()
    else {
        panic!()
    };
    let Geometry::Box {
        children: markers, ..
    } = elements[1].geometry.as_ref()
    else {
        panic!()
    };
    let Geometry::Polyline { points, .. } = curve[0].geometry.as_ref() else {
        panic!()
    };
    for (point, marker) in points.iter().zip(markers) {
        for axis in 0..2 {
            assert!(
                (elements[0].position.values[axis]
                    + curve[0].position.values[axis]
                    + point.values[axis]
                    - elements[1].position.values[axis]
                    - marker.position.values[axis])
                    .abs()
                    < 1e-9
            );
        }
    }
    assert_eq!(points[0].unit, Unit::Length);
}

#[test]
fn plot_errors_report_statistics_and_scaling_reasons() {
    for (source, reason) in [
        (
            "let p=Histogram(values=[7],x_scale=xs,y_scale=ys)",
            "outside the bin edges",
        ),
        (
            "let p=Histogram(values=[0],bins=2,edges=[-4,4],x_scale=xs,y_scale=ys)",
            "not both",
        ),
        (
            "let p=DensityPlot(values=[1,1],x_scale=xs,y_scale=ys)",
            "constant dataset",
        ),
        (
            "let p=NormalCurve(std_dev=0,x_scale=xs,y_scale=ys)",
            "std_dev",
        ),
        (
            "let p=LinePlot(data=[{x=1,y=0},{x=0,y=1}],interpolation=\"monotone\",x_scale=xs,y_scale=ys)",
            "strictly increasing",
        ),
        (
            "let p=BandPlot(data=[{x=0,low=0.8,high=0.2}],x_scale=xs,y_scale=ys)",
            "low <= high",
        ),
        ("let p=Legend(labels=[\"x\"],colors=[])", "exactly one"),
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", format!("{SCALES}\n{source}\nAnimation()"));
        let error = Engine::default()
            .prepare("main.sanim", &files, &Default::default())
            .unwrap_err();
        assert!(error.message.contains(reason), "{error}");
        assert!(error.span.line > 0);
    }
}

#[test]
fn histogram_empty_lists_and_range_inference_have_defined_behavior() {
    let prepared = prepare(
        r#"
let domain=DataRange([2,4,6],padding=0.1,include_zero=true)
assert(abs(domain[0] + 0.6) < 0.0001)
assert(abs(domain[1] - 6.6) < 0.0001)
let empty=Histogram(values=[],x_scale=xs,y_scale=ys,normalization="density")
Animation(children=[empty])
"#,
    );
    assert!(
        matches!(prepared.animation.find("empty").unwrap().geometry.as_ref(),Geometry::Box {children, ..}if children.is_empty())
    );
}

#[test]
fn native_statistics_reserve_cumulative_work_and_feed_performance_advice() {
    let mut files = MemorySources::default();
    files.insert("main.sanim",format!("{SCALES}\nrepeat 3 {{let curve=DensityPlot(values=[-2,-1.5,-1,-0.5,0,0.5,1,1.5,2,2.5],bandwidth=0.6,points=32,x_scale=xs,y_scale=ys)}}\nAnimation()"));
    let mut engine = Engine::new(Default::default());
    engine.limits.max_operations = 1000;
    engine.limits.max_native_work = 1000;
    let error = engine
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert_eq!(error.code, "S303");
    assert!(error.message.contains("native work"));
    let report = engine.lint("main.sanim", &files).unwrap();
    assert!(
        report
            .diagnostics
            .iter()
            .any(|warning| warning.code == "P101" && warning.message.contains("Gaussian"))
    );
    files.insert("main.sanim",format!("{SCALES}\nlet curve=DensityPlot(values=[-2,-1,0,1,2],bandwidth=0.6,points=32,x_scale=xs,y_scale=ys)\nAnimation(children=[curve])"));
    engine.limits.max_operations = 10_000;
    engine.limits.max_native_work = 10_000;
    engine.limits.warn_operations = 100;
    engine.limits.warn_prepare_micros = 0;
    let prepared = engine
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    assert!(prepared.statistics.operations > 160);
    assert!(
        prepared
            .performance
            .iter()
            .any(|warning| warning.code == "P004")
    );
}
