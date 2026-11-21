use sanim_core::{Color, Fraction, Property, Value};
use sanim_lang::{Engine, MemorySources};

fn prepare(source: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", format!("{source}\nAnimation()"));
    Engine::default().prepare("main.sanim", &sources, &Default::default())
}
fn color(prepared: &sanim_lang::Prepared, name: &str) -> [u8; 4] {
    let Value::Color(color) = &prepared.bindings[name] else {
        panic!("Color")
    };
    color.rgba8().unwrap()
}

#[test]
fn constructors_conversions_and_perceptual_adjustments_are_typed_and_pure() {
    let prepared = prepare(
        r#"
let red=Rgb(1,0,0)
fn shade<T:Color>(value:T):Color {return value.mix(Color("white"),0.2)}
let generic=shade(red)
assert(Rgb(0.1,0.2,0.3).to_rgb().red==0.1)
assert(Hsl(30deg,0.4,0.5,alpha=0.3).to_hsl().alpha==0.3)
assert(Oklch(0.7,0.1,25deg,alpha=0.3).rotate_hue(10deg).to_oklch().alpha==0.3)
assert(Rgb(0.1,0.2,0.3,alpha=0.3).over(Color("transparent"))==Rgb(0.1,0.2,0.3,alpha=0.3))
let green=Hsl(120deg,1,0.5)
let blue=Hsv(-120deg,1,1)
let tiny=Hsv(1deg/1000000000000000000,1,1)
let huge=Hsv(1000000000000000030deg,1,1)
assert(huge==Hsv(310deg,1,1))
let lab=red.to_oklab()
let red_again=Oklab(lab.lightness,lab.a,lab.b)
let lch=green.to_oklch()
let green_again=Oklch(lch.lightness,lch.chroma,lch.hue)
let purple=Color("rebeccapurple")
let adjusted=purple.with_lightness(0.7).with_chroma(0.1).with_hue(35deg).rotate_hue(20deg)
let changed=adjusted.to_oklch()
assert(abs(changed.lightness-0.7)<0.0001)
assert(abs(changed.chroma-0.1)<0.0001)
assert(abs(changed.hue-55deg)<0.01deg)
assert(Color("black").contrast(Color("white"))>20.99)
let hsla=Rgb(0.1,0.2,0.3,alpha=0.25).to_hsl()
assert(hsla.alpha==0.25)
assert(Color("black").mix(Color("white"),0)==Color("black"))
assert(Color("black").mix(Color("white"),1)==Color("white"))
"#,
    )
    .unwrap();
    assert_eq!(color(&prepared, "red"), [255, 0, 0, 255]);
    assert_eq!(color(&prepared, "green"), [0, 255, 0, 255]);
    assert_eq!(color(&prepared, "blue"), [0, 0, 255, 255]);
    assert_eq!(color(&prepared, "tiny"), [255, 0, 0, 255]);
    assert_eq!(color(&prepared, "red_again"), [255, 0, 0, 255]);
    assert_eq!(color(&prepared, "green_again"), [0, 255, 0, 255]);
}

#[test]
fn mixing_spaces_alpha_and_gamut_are_explicit() {
    let prepared = prepare(
        r#"
let perceptual=Color("black").mix(Color("white"),0.5)
let encoded=Color("black").mix(Color("white"),0.5,space="srgb")
let linear=Color("black").mix(Color("white"),0.5,space="linear-rgb")
let transparent=Rgb(1,0,0,alpha=0).mix(Color("blue"),0.5,space="oklch")
let mapped=Oklch(0.65,2,35deg)
let mapped_lch=mapped.to_oklch()
assert(abs(mapped_lch.lightness-0.65)<0.0001)
assert(abs(mapped_lch.hue-35deg)<0.01deg)
let over=Rgb(1,0,0,alpha=0.5).over(Color("blue"))
"#,
    )
    .unwrap();
    assert_eq!(color(&prepared, "perceptual"), [99, 99, 99, 255]);
    assert_eq!(color(&prepared, "encoded"), [128, 128, 128, 255]);
    assert_eq!(color(&prepared, "linear"), [188, 188, 188, 255]);
    assert_eq!(color(&prepared, "transparent"), [0, 0, 255, 128]);
    assert_eq!(color(&prepared, "over"), [188, 0, 188, 255]);
    for (source, reason) in [
        ("let c=Rgb(1.000000001,0,0)", "0..=1"),
        ("let c=Color().with_alpha(-0.000000001)", "0..=1"),
        ("let c=Oklch(0.5,-0.1,0deg)", "nonnegative"),
        ("let c=Oklch(0.5,1,0deg,gamut=\"error\")", "outside"),
        ("let c=Rgb(0,0,0,alpha=0.5).contrast(Color())", "opaque"),
        ("let c=Hsl(30,1,0.5)", "Angle"),
    ] {
        let error = prepare(source).unwrap_err();
        assert!(error.message.contains(reason), "{source}: {error}");
    }
}

#[test]
fn reactive_colors_and_nested_record_projections_use_the_native_graph() {
    let source = r#"
mut hue=0deg
mut amount=0
sig tint=Oklch(0.65,0.12,hue).mix(Color("white"),amount).with_alpha(0.8)
sig lightness=tint.to_oklch().lightness
sig state={outer={value=lightness}}
let box=<Box w-40 h-40 fill={tint} opacity={state.outer.value}/>
Animation(children=[box])
<Animation absolute flow="sequence">{animate(hue,to=180deg,duration=1s,ease="linear")}</Animation>
<Animation absolute flow="sequence">{animate(amount,to=0.5,duration=1s,ease="linear")}</Animation>
wait(1s)
"#;
    let prepared = prepare(source).unwrap();
    let root = prepared.animation.find("box").unwrap();
    let at = |time| {
        prepared
            .animation
            .sample_property(root, Property::Fill, time)
            .unwrap()
    };
    let start = at(0.into());
    let end = at(1.into());
    assert_ne!(start, end);
    assert_eq!(
        at(0.into()),
        start,
        "reverse seek must not retain stale native color state"
    );
    let midpoint = at(Fraction::new(1u64, 2u64));
    let Value::Color(midpoint) = midpoint else {
        panic!("Color")
    };
    assert_eq!(midpoint.rgba8().unwrap()[3], 204);
    let opacity = prepared
        .animation
        .sample_property(root, Property::Opacity, 0.into())
        .unwrap()
        .number()
        .unwrap()
        .value;
    assert!((sanim_core::math::to_f64(opacity).unwrap() - 0.65).abs() < 0.001);
    assert_eq!(
        Color::parse("transparent").unwrap().rgba8().unwrap(),
        [0; 4]
    );
}
