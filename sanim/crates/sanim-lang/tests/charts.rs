use sanim_core::{Fraction, Geometry, Unit, Value};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

fn files(text: &str) -> MemorySources {
    let mut files = MemorySources::default();
    files.insert(
        "font.ttf",
        include_bytes!("../../../tests/fixtures/DejaVuSansCondensed-subset.ttf"),
    );
    files.insert("points.csv", "x,y\n0.8,1\n1.0,2\n1.2,3\n");
    files.insert("main.sanim", text);
    files
}
fn prepare(text: &str) -> sanim_lang::Prepared {
    Engine::default()
        .prepare("main.sanim", &files(text), &BTreeMap::new())
        .unwrap_or_else(|e| panic!("{e}"))
}
const SCALES: &str = r#"
let xs=LinearScale(domain=Vec2(0.78,1.28),range=Vec2(-275.4pt,308.8125pt))
let ys=LinearScale(domain=Vec2(0,4),range=Vec2(200pt,-200pt))
"#;
#[test]
fn scales_grid_axis_scatter_and_boxplot_share_the_same_coordinates() {
    let p = prepare(&format!(
        r#"{SCALES}
let font=read<Font>("font.ttf")
type Sample={{x:Number,y:Number}}
let data=read<List<Sample>>("points.csv")
let dots=Scatter(id="dots",data=data,x_scale=xs,y_scale=ys,keys=["a","b","c"])
let grid=Grid(id="grid",scale=xs,ticks=[0.8,1.0,1.2],spans=[Vec2(-100pt,100pt)],highlight=[1.0])
let axis=Axis(id="axis",scale=xs,ticks=[0.8,1.0,1.2],format="percent",font=font)
let interval_box=BoxPlot(id="box",summary={{low=0.8,q1=0.9,middle=1.0,q3=1.1,high=1.2}},scale=xs,height=16pt)
add(dots)
add(grid)
add(axis)
play(ExpandX(interval_box),duration=400ms,ease="smootherstep")
"#
    ));
    let dots = p.animation.find("dots").unwrap();
    let Geometry::Group { children } = dots.geometry.as_ref() else {
        panic!()
    };
    assert_eq!(children.len(), 3);
    assert_eq!(children[1].id, "dots/point/b");
    assert_eq!(children[1].position.unit, Unit::Length);
    assert_eq!(children[1].position.values[1], 0.);
    let median = p.animation.find("box/median").unwrap();
    assert_eq!(children[1].position.values[0], median.position.values[0]);
    let frame = p
        .animation
        .layout(
            Fraction::new(1u64, 5u64),
            Fraction::from(800),
            Fraction::from(600),
        )
        .unwrap();
    let range = frame.iter().find(|e| e.id == "box").unwrap();
    assert_eq!(range.horizontal.scale, Fraction::new(1u64, 2u64));
    assert!(
        (sanim_core::math::to_f64(range.horizontal.origin.value).unwrap()
            - 400.
            - median.position.values[0])
            .abs()
            < 1e-9
    );
    let label = p.animation.find("axis/label/1").unwrap();
    assert!(matches!(label.geometry.as_ref(),Geometry::Text{text,..}if text=="100%"));
}
#[test]
fn native_quantiles_match_linear_interpolation_and_do_not_modify_samples() {
    let p = prepare(
        "let values=[1,2,3,4]\nlet stats=BoxSummary(values)\nassert(stats.low==1)\nassert(stats.q1==1.75)\nassert(stats.middle==2.5)\nassert(stats.q3==3.25)\nassert(stats.high==4)\nwait(1s)",
    );
    assert!(matches!(&p.bindings["values"],Value::List(v)if v.len()==4));
}
#[test]
fn native_charts_fail_with_actionable_messages() {
    for (script, message) in [
        (
            "let bad=LinearScale(domain=Vec2(1,1),range=Vec2(0pt,100pt))",
            "domain must increase",
        ),
        (
            "let bad=LinearScale(domain=Vec2(0,1),range=Vec2(0pt,0pt))",
            "nonzero length",
        ),
        (
            "let bad=BoxPlot(scale=xs,summary={low=0.8,q1=1.1,middle=1.0,q3=1.2,high=1.25})",
            "low <= q1",
        ),
        (
            "let bad=Grid(scale=xs,ticks=[1.0,0.8],spans=[Vec2(0pt,100pt)])",
            "strictly increasing",
        ),
        (
            "let bad=Grid(scale=xs,ticks=[0.7],spans=[Vec2(0pt,100pt)])",
            "outside domain",
        ),
        (
            "let font=read<Font>(\"font.ttf\");let bad=Axis(scale=xs,ticks=[0.8,0.8001],font=font)",
            "labels overlap",
        ),
    ] {
        let error = Engine::default()
            .prepare(
                "main.sanim",
                &files(&format!("{SCALES}\n{script}\nwait(1s)")),
                &BTreeMap::new(),
            )
            .unwrap_err();
        assert!(error.message.contains(message), "{error}");
        assert_eq!(error.source, "main.sanim");
        assert!(error.span.line > 0);
    }
}
#[test]
fn scatter_schema_and_constructor_protection_are_static() {
    for script in [
        "let Grid=1",
        r#"type Wrong={x:String,y:Number}
let rows=read<List<Wrong>>("points.csv")
let dots=Scatter(data=rows,x_scale=xs,y_scale=ys)"#,
    ] {
        assert!(
            Engine::default()
                .prepare(
                    "main.sanim",
                    &files(&format!("{SCALES}\n{script}")),
                    &BTreeMap::new()
                )
                .is_err()
        );
    }
}
