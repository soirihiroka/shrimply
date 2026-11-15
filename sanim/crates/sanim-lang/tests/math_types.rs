use sanim_core::{Fraction, Type, Unit, Value};
use sanim_lang::{Engine, MemorySources, Prepared};
use std::collections::BTreeMap;

fn run(text: &str) -> Result<Prepared, sanim_core::Diagnostic> {
    run_in(text, 1920, 1080)
}
fn run_in(text: &str, width: u32, height: u32) -> Result<Prepared, sanim_core::Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", format!("{text}\nAnimation()\n"));
    Engine::new(sanim_core::Viewport {
        width,
        height,
        dpi: 72,
        ..sanim_core::Viewport::default()
    })
    .prepare("main.sanim", &files, &BTreeMap::new())
}

#[test]
fn native_vector_dimensions_and_components() {
    let p=run("let a: List<Number,2> = [1, 2]\nlet b: List<Number,3> = [1, 2, 3]\nlet c: List<Number,4> = [1, 2, 3, 4]\nlet z = b[2]\nlet w = c[3]\n").unwrap();
    assert_eq!(
        p.bindings["a"].ty(),
        Type::List(
            Box::new(Type::Numeric(Unit::Number)),
            sanim_core::ListLength::Exact(2)
        )
    );
    assert_eq!(p.bindings["z"].number().unwrap().value, Fraction::from(3));
    assert_eq!(p.bindings["w"].number().unwrap().value, Fraction::from(4));
}
#[test]
fn vector_defaults_use_the_supplied_unit() {
    let p = run("let value = [10pt, 0pt, 0pt]\n").unwrap();
    let (v, unit) = p.bindings["value"].vector().unwrap();
    assert_eq!(unit, Unit::Length);
    assert_eq!(v, &[10.0, 0.0, 0.0]);
}
#[test]
fn vector_add_subtract_and_scale() {
    let p =
        run("let a = [1pt, 2pt]+ [3pt, 4pt]\nlet b = 2 * a - [1pt, 1pt]\nlet c = b / 2\n").unwrap();
    assert_eq!(p.bindings["c"].vector().unwrap().0, &[3.5, 5.5]);
}
#[test]
fn vectors_cannot_mix_dimensions() {
    assert_eq!(
        run("let value = [1, 2]+ [1, 2, 3]\n").unwrap_err().code,
        "S106"
    );
}
#[test]
fn vectors_cannot_mix_component_units() {
    assert_eq!(run("let value = [1pt, 2s]\n").unwrap_err().code, "S106");
}
#[test]
fn typed_vector_parameter_preserves_units() {
    let p=run("fn double(v: List<Length,3>): List<Length,3> { return v * 2 }\nlet result = double([1pt, 2pt, 3pt])\n").unwrap();
    assert_eq!(p.bindings["result"].vector().unwrap().0[2], 6.0);
}
#[test]
fn vector_dot_product() {
    let p = run("let result = dot([1, 2, 3], [4, 5, 6])\n").unwrap();
    assert_eq!(
        p.bindings["result"].number().unwrap().value,
        Fraction::from(32)
    );
}
#[test]
fn dot_rejects_unrepresented_squared_units() {
    assert!(run("let result = dot([1pt, 2pt], [1pt, 2pt])\n").is_err());
}
#[test]
fn fraction_has_its_own_type_and_exact_arithmetic() {
    let p=run("let result: Fraction = Fraction(1, 3) + Fraction(1, 6)\nlet numerator = result.numerator\nlet denominator = result.denominator\n").unwrap();
    assert_eq!(
        p.bindings["result"],
        Value::Fraction(Fraction::new(1u64, 2u64))
    );
    assert_eq!(
        p.bindings["numerator"].number().unwrap().value,
        Fraction::from(1)
    );
    assert_eq!(
        p.bindings["denominator"].number().unwrap().value,
        Fraction::from(2)
    );
}
#[test]
fn fraction_sign_and_scalar_promotion() {
    let p=run("let value = -Fraction(2, 3) + 1\nlet offset = [3pt, 6pt]* Fraction(1, 3)\nassert(value == Fraction(1, 3))\n").unwrap();
    assert_eq!(p.bindings["offset"].vector().unwrap().0[0], 1.0);
}
#[test]
fn fraction_denominator_zero_fails() {
    assert!(
        run("let value = Fraction(1, 0)\n")
            .unwrap_err()
            .message
            .contains("division by zero")
    );
}
#[test]
fn fraction_requires_integer_parts() {
    assert!(run("let value = Fraction(1.5, 3)\n").is_err());
}
#[test]
fn matrix_identity_and_vector_transform() {
    let p=run("let identity = Matrix4()\nlet transformed = identity * [1, 2, 3, 1]\nassert(transformed == [1, 2, 3, 1])\n").unwrap();
    assert!(matches!(p.bindings["identity"], Value::Matrix4(_)));
}
#[test]
fn matrix_multiply_inverse_and_transpose() {
    let p=run("let m = Matrix4([[2,0,0,4],[0,3,0,6],[0,0,4,8],[0,0,0,1]])\nlet i = inverse(m)\nassert(m * i == Matrix4())\nassert(transpose(transpose(m)) == m)\nlet result = m * [1, 2, 3, 1]\n").unwrap();
    assert_eq!(
        p.bindings["result"].vector().unwrap().0,
        &[6.0, 12.0, 20.0, 1.0]
    );
}
#[test]
fn matrix_shape_is_checked() {
    assert!(
        run("let m = Matrix4([[1,2],[3,4]])\n")
            .unwrap_err()
            .message
            .contains("four")
    );
}
#[test]
fn matrix_singular_inverse_fails() {
    assert!(
        run("let m = inverse(Matrix4([[0,0,0,0],[0,0,0,0],[0,0,0,0],[0,0,0,0]]))\n")
            .unwrap_err()
            .message
            .contains("singular")
    );
}
#[test]
fn matrix_requires_explicit_homogeneous_component() {
    assert_eq!(
        run("let v = Matrix4() * [1, 2, 3]\n").unwrap_err().code,
        "S106"
    );
}
#[test]
fn native_math_types_are_reflectable() {
    let p=run("type Parameters = { direction: List<Number,3>, transform: Matrix4, ratio: Fraction }\nlet parameters: Parameters=reflect({ direction=[1, 0, 0], transform=Matrix4(), ratio=Fraction(1,3) })\n").unwrap();
    assert!(p.reflected[0].ty.reflectable());
}
#[test]
fn alignment_is_independent_of_vector_math() {
    let p=run_in("Animation(children=[Box(id=\"box\", width=20pt, height=10pt, position=[-5pt, 3pt], x_align=Align.End, y_align=Align.Start, fill=Color(\"light1\"))],duration=1s)\n",100,100).unwrap();
    let elements = p
        .animation
        .layout(Fraction::from(0), Fraction::from(100), Fraction::from(100))
        .unwrap();
    assert_eq!(elements[0].position.values, [75.0, 3.0]);
}
#[test]
fn point_type_is_removed() {
    for constructor in ["Point2", "Point", "point2", "point", "Vec2", "Vec3", "Vec4"] {
        let diagnostic = run(&format!("let value = {constructor}(1pt, 2pt)\n")).unwrap_err();
        assert!(
            diagnostic.message.contains("unknown name"),
            "{diagnostic:?}"
        );
    }
    for ty in ["Point2", "Point", "Vec2", "Vec3", "Vec4"] {
        assert!(run(&format!("let value: {ty} = [1pt, 2pt]\n")).is_err());
    }
}
#[test]
fn lowercase_constructor_aliases_are_not_defined() {
    assert!(run("let value = vec2(1,2)\n").is_err());
}

#[test]
fn local_geometry_vertices_require_length_vectors() {
    for text in [
        "let edge=Line(end= [1s, 2s])\n",
        "let shape=Polygon(points= [[0, 0],[1, 0],[0, 1]])\n",
    ] {
        assert_eq!(run(text).unwrap_err().code, "S106");
    }
}
