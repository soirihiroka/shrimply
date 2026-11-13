use sanim_core::{Fraction, Value};
use sanim_lang::{Engine, MemorySources, Prepared};
use std::collections::BTreeMap;

fn run(source: &str) -> Prepared {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}\n{source}"))
}

fn number(prepared: &Prepared, name: &str) -> Fraction {
    prepared.bindings[name].number().unwrap().value
}

#[test]
fn signals_follow_writes_while_let_and_mut_initializers_take_snapshots() {
    let p = run("mut a=1\nsig b=a*2\nlet old=b\nmut saved=b\na=2\nlet current=b\n");
    assert_eq!(number(&p, "a"), Fraction::from(2));
    assert_eq!(number(&p, "b"), Fraction::from(4));
    assert_eq!(number(&p, "old"), Fraction::from(2));
    assert_eq!(number(&p, "saved"), Fraction::from(2));
    assert_eq!(number(&p, "current"), Fraction::from(4));
}

#[test]
fn chained_signals_and_assignment_rhs_read_the_previous_state() {
    let p =
        run("mut a=1\nsig twice=a*2\nsig next=twice+1\nlet before=next\na=next\nlet after=next\n");
    assert_eq!(number(&p, "before"), Fraction::from(3));
    assert_eq!(number(&p, "a"), Fraction::from(3));
    assert_eq!(number(&p, "twice"), Fraction::from(6));
    assert_eq!(number(&p, "after"), Fraction::from(7));
}

#[test]
fn nested_blocks_and_bounded_loops_write_the_same_captured_cell() {
    let p = run(
        "mut count=0\nsig doubled=count*2\nrepeat 4 {if true {count=count+1}}\nfor item in range(3) {count=count+item}\nlet observed=doubled\n",
    );
    assert_eq!(number(&p, "count"), Fraction::from(7));
    assert_eq!(number(&p, "observed"), Fraction::from(14));
}

#[test]
fn helpers_write_captured_mutables_and_return_materialized_values() {
    let p = run(
        "mut count=1\nsig doubled=count*2\nfn advance():Number{count=count+1;return doubled}\nlet first=advance()\nlet second=advance()\n",
    );
    assert_eq!(number(&p, "count"), Fraction::from(3));
    assert_eq!(number(&p, "first"), Fraction::from(4));
    assert_eq!(number(&p, "second"), Fraction::from(6));
    assert_eq!(number(&p, "doubled"), Fraction::from(6));
}

#[test]
fn function_local_cells_are_fresh_and_parameters_are_value_snapshots() {
    let p = run(
        "mut count=1\nfn local(value:Number):Number{mut scratch=value;sig doubled=scratch*2;scratch=scratch+1;return doubled}\nlet first=local(count)\ncount=4\nlet second=local(count)\nlet third=local(1)\n",
    );
    assert_eq!(number(&p, "first"), Fraction::from(4));
    assert_eq!(number(&p, "second"), Fraction::from(10));
    assert_eq!(number(&p, "third"), Fraction::from(4));
    assert_eq!(number(&p, "count"), Fraction::from(4));
}

#[test]
fn dependencies_read_through_pure_helpers_invalidate_signals() {
    let p = run(
        "mut count=1\nfn current():Number{return count}\nsig doubled=current()*2\nlet before=doubled\ncount=3\nlet after=doubled\n",
    );
    assert_eq!(number(&p, "before"), Fraction::from(2));
    assert_eq!(number(&p, "after"), Fraction::from(6));
}

#[test]
fn signals_update_their_dependencies_when_a_helper_switches_branches() {
    let p = run(
        "mut choose_first=true\nmut first=1\nmut second=2\nfn selected():Number{if choose_first{return first}else{return second}}\nsig chosen=selected()\nlet before=chosen\nchoose_first=false\nlet switched=chosen\nfirst=9\nlet unrelated=chosen\nsecond=5\nlet after=chosen\n",
    );
    assert_eq!(number(&p, "before"), Fraction::from(1));
    assert_eq!(number(&p, "switched"), Fraction::from(2));
    assert_eq!(number(&p, "unrelated"), Fraction::from(2));
    assert_eq!(number(&p, "after"), Fraction::from(5));
}

#[test]
fn imported_members_are_live_and_local_import_reads_are_snapshots() {
    let mut files = MemorySources::default();
    files.insert(
        "theme.sanim",
        "mut count=1\nsig doubled=count*2\nfn advance():Unit{count=count+1}\n",
    );
    files.insert("main.sanim", "import \"theme\" as theme\nlet saved=theme.count\nlet before=theme.doubled\ntheme.advance()\nlet current=theme.count\nlet after=theme.doubled\n");
    let p = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(number(&p, "saved"), Fraction::from(1));
    assert_eq!(number(&p, "before"), Fraction::from(2));
    assert_eq!(number(&p, "current"), Fraction::from(2));
    assert_eq!(number(&p, "after"), Fraction::from(4));
}

#[test]
fn pure_expression_caches_use_current_mutable_and_signal_values() {
    let p = run(
        "mut amount=1\nsig mirrored=amount\nlet before=abs(amount)\nlet signal_before=abs(mirrored)\namount=-3\nlet after=abs(amount)\nlet signal_after=abs(mirrored)\n",
    );
    assert_eq!(number(&p, "before"), Fraction::from(1));
    assert_eq!(number(&p, "signal_before"), Fraction::from(1));
    assert_eq!(number(&p, "after"), Fraction::from(3));
    assert_eq!(number(&p, "signal_after"), Fraction::from(3));
}

#[test]
fn optional_mutables_keep_the_declared_type_after_none_initialization() {
    let p = run(
        "mut maybe:Option<Number>=None\nsig projected=unwrap(maybe,0)\nmaybe=Some(3)\nlet present=projected\nmaybe=None\nlet absent=projected\n",
    );
    assert_eq!(number(&p, "present"), Fraction::from(3));
    assert_eq!(number(&p, "absent"), Fraction::from(0));
    assert_eq!(p.bindings["maybe"], Value::None);
}

#[test]
fn duration_planning_preserves_writes_in_helpers_arguments_branches_and_loops() {
    for (source, expected) in [
        (
            "mut count=1\nfn change():Bool{count=2;return true}\nlet ignored=false and change()\nwait(count*100ms)",
            Fraction::new(1u64, 10u64),
        ),
        (
            "mut count=1\nfn change():Bool{count=2;return false}\nlet ignored=true or change()\nwait(count*100ms)",
            Fraction::new(1u64, 10u64),
        ),
        (
            "mut count=1\nsig delay=count*100ms\ncount=2\nwait(delay)",
            Fraction::new(1u64, 5u64),
        ),
        (
            "mut count=1\nsig delay=count*100ms\nfn advance():Unit{count=count+1}\nadvance()\nwait(delay)",
            Fraction::new(1u64, 5u64),
        ),
        (
            "mut count=1\nsig delay=count*100ms\nfn advance():Number{count=count+1;return 0}\nlet ignored=advance()\nwait(delay)",
            Fraction::new(1u64, 5u64),
        ),
        (
            "mut count=1\nsig delay=count*100ms\nfn advance():Number{count=count+1;return 0}\nfn ignore(value:Number):Unit{let unused=value}\nrepeat 3 {ignore(advance());wait(delay)}",
            Fraction::new(9u64, 10u64),
        ),
        (
            "mut count=1\nsig delay=count*100ms\nfn advance():Number{count=count+1;return 0}\nfn phase(unused:Number=advance()):Unit{wait(delay)}\nphase()",
            Fraction::new(1u64, 5u64),
        ),
        (
            "mut count=0\nsig delay=count*100ms\ntimer 3 every 1s {count=count+1;wait(delay)}\nwait(delay)",
            Fraction::new(33u64, 10u64),
        ),
        (
            "mut count=1\nsig delay=count*100ms\nif true {count=4}else{count=8}\nwait(delay)",
            Fraction::new(2u64, 5u64),
        ),
        (
            "mut count=1\nsig delay=count*100ms\nfor item in range(3) {count=count+1;wait(delay)}",
            Fraction::new(9u64, 10u64),
        ),
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        let mut engine = Engine::default();
        let planned = engine
            .duration("main.sanim", &files, &BTreeMap::new())
            .unwrap_or_else(|error| panic!("{error}\n{source}"));
        let full = engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_or_else(|error| panic!("{error}\n{source}"));
        assert_eq!(planned.duration, expected, "{source}");
        assert_eq!(full.animation.duration, expected, "{source}");
    }
}

#[test]
fn duration_planning_skips_unrelated_signal_assets() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "sig unrelated=read<List<Number>>(\"missing.json\")\nmut count=1\nsig delay=count*100ms\ncount=2\nwait(delay)\n");
    let planned = Engine::default()
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(planned.duration, Fraction::new(1u64, 5u64));
    assert!(!planned.dependencies.contains_key("missing.json"));
}

#[test]
fn suffix_restoration_preserves_cells_and_signal_memos_without_duplicate_inputs() {
    let prefix = "let seed=reflect(1)\nmut count=seed\nsig doubled=count*2\nlet old=doubled\nwait(1s)\ncount=2\nlet observed=doubled\n";
    let mut files = MemorySources::default();
    let mut engine = Engine::default();
    files.insert("main.sanim", format!("{prefix}wait(1s)\n"));
    engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    files.insert("main.sanim", format!("{prefix}wait(2s)\n"));
    let warm = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let fresh = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert!(warm.statistics.restored_statements >= 7);
    assert_eq!(warm.fingerprint, fresh.fingerprint);
    assert_eq!(number(&warm, "old"), Fraction::from(2));
    assert_eq!(number(&warm, "doubled"), Fraction::from(4));
    assert_eq!(warm.reflected.len(), 1);
    let changed = engine
        .prepare_json(
            "main.sanim",
            &files,
            &BTreeMap::from([("main.sanim::seed".into(), serde_json::json!(3))]),
        )
        .unwrap();
    assert_eq!(number(&changed, "old"), Fraction::from(6));
    assert_eq!(number(&changed, "observed"), Fraction::from(4));
    assert_eq!(changed.reflected.len(), 1);
}

#[test]
fn only_mutable_bindings_accept_assignment_and_the_type_is_fixed() {
    for source in [
        "let value=1\nvalue=2",
        "sig value=1\nvalue=2",
        "mut value=1\nvalue=\"wrong\"",
        "mut value=1pt\nvalue=1s",
        "mut value:Option<Number>=None\nvalue=Some(\"wrong\")",
        "mut Color=1",
        "missing=2",
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        let error = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err();
        assert_eq!(error.source, "main.sanim", "{error}");
        assert!(!error.message.is_empty());
        assert!(error.span.column > 0, "{error}");
    }
}

#[test]
fn signals_reject_transitive_writes_scheduling_assertions_and_cycles() {
    for source in [
        "mut count=1\nfn advance():Number{count=count+1;return count}\nsig value=advance()",
        "fn phase():Number{wait(1ms);return 1}\nsig value=phase()",
        "fn checked():Number{assert(true);return 1}\nsig value=checked()",
        "sig value=reflect(1)",
        "sig value=current()\nfn current():Number{return value}",
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        let error = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err();
        assert_eq!(error.source, "main.sanim", "{error}");
        assert!(
            error.message.contains("sig") || error.message.contains("cycle"),
            "{error}"
        );
    }
}

#[test]
fn background_cell_animation_drives_shared_retained_dimensions_without_advancing_main_time() {
    let source = "mut phase=0deg\nsig size=100pt+sin(phase)*25pt\nlet saved=size\nlet rectangle=Rect(id=\"rectangle\",width=size,height=size)\nlet circle=Circle(id=\"circle\",radius=size)\nlet frozen=Rect(id=\"frozen\",width=saved,height=saved)\nadd(rectangle)\nadd(circle)\nadd(frozen)\nbackground {animate(phase,to=1800deg,duration=10s)}\nwait(10s)\n";
    let p = run(source);
    assert_eq!(p.animation.duration, Fraction::from(10));
    assert_eq!(
        p.animation
            .reactive
            .binding("rectangle", sanim_core::Property::Width),
        p.animation
            .reactive
            .binding("circle", sanim_core::Property::Radius)
    );
    for (time, size) in [
        (Fraction::from(0), 100),
        (Fraction::new(1u64, 2u64), 125),
        (Fraction::new(3u64, 2u64), 75),
        (Fraction::from(10), 100),
    ] {
        let frame = p.animation.sample(time).unwrap();
        assert!(
            matches!(frame[0].geometry.as_ref(),sanim_core::Geometry::Rectangle {width,height, .. } if *width==Fraction::from(size)&&*height==Fraction::from(size))
        );
        assert!(
            matches!(frame[1].geometry.as_ref(),sanim_core::Geometry::Circle{radius} if *radius==Fraction::from(size))
        );
        assert!(
            matches!(frame[2].geometry.as_ref(),sanim_core::Geometry::Rectangle {width,height, .. } if *width==Fraction::from(100)&&*height==Fraction::from(100))
        );
    }
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    let planned = Engine::default()
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(planned.duration, p.animation.duration);
}

#[test]
fn background_jobs_extend_duration_by_their_latest_end_instead_of_their_sum() {
    let p = run(
        "mut phase=0deg\nmut amount=0\nbackground {animate(phase,to=1800deg,duration=10s)}\nbackground {animate(amount,to=1,duration=4s)}\nwait(2s)\n",
    );
    assert_eq!(p.animation.duration, Fraction::from(10));
    assert_eq!(number(&p, "phase"), Fraction::from(360));
    assert_eq!(number(&p, "amount"), Fraction::new(1u64, 2u64));
}

#[test]
fn foreground_animate_advances_time_and_conflicting_cell_writes_are_rejected() {
    let p = run(
        "mut amount=0\nsig size=amount+1\nadd(Circle(id=\"circle\",scale=size))\nanimate(amount,to=1,duration=2s)\nwait(1s)\n",
    );
    assert_eq!(p.animation.duration, Fraction::from(3));
    assert_eq!(
        p.animation.sample(Fraction::from(1)).unwrap()[0].scale,
        Fraction::new(3u64, 2u64)
    );
    for source in [
        "mut amount=0\nbackground {animate(amount,to=1,duration=2s)}\nbackground {animate(amount,to=2,duration=2s)}",
        "mut amount=0\nbackground {animate(amount,to=1,duration=2s)}\nwait(1s)\namount=2",
        "mut amount=0\nanimate(amount,to=1,duration=0s)",
        "mut amount=0\nanimate(amount,to=1,duration=7201s)",
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        assert!(
            Engine::default()
                .prepare("main.sanim", &files, &BTreeMap::new())
                .is_err(),
            "{source}"
        );
    }
}

#[test]
fn malformed_host_union_list_payloads_are_fully_validated() {
    use sanim_core::{List, Number, Type, Unit};
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let initial_rows:List<Number>|List<Bool>=[1]\nlet editable_rows=reflect(initial_rows)\n",
    );
    let malformed = Value::List(List::typed(
        Type::Numeric(Unit::Number),
        vec![
            Value::Number(Number::scalar(1)),
            Value::String("bad item".into()),
        ],
    ));
    let error = Engine::default()
        .prepare(
            "main.sanim",
            &files,
            &BTreeMap::from([("editable_rows".into(), malformed)]),
        )
        .unwrap_err();
    assert_eq!(error.code, "S302");
    assert!(error.message.contains("String"), "{error}");
}

#[test]
fn retained_dimensions_reflow_rows_but_row_owned_positions_are_rejected() {
    let p = run(
        "mut amount=10pt\nsig extent=amount\nlet first=Rect(id=\"first\",width=extent,height=10pt)\nlet second=Rect(id=\"second\",width=10pt,height=10pt)\nadd(Row(id=\"row\",children=[first,second],gap=5pt))\nbackground {animate(amount,to=20pt,duration=2s)}\nwait(2s)",
    );
    let (width, height) = p.animation.logical_size();
    let before = p
        .animation
        .layout(Fraction::from(0), width, height)
        .unwrap();
    let after = p
        .animation
        .layout(Fraction::from(2), width, height)
        .unwrap();
    let sanim_core::Geometry::Group { children: before } = before[0].geometry.as_ref() else {
        panic!("row group expected")
    };
    let sanim_core::Geometry::Group { children: after } = after[0].geometry.as_ref() else {
        panic!("row group expected")
    };
    assert!((after[1].position.values[0] - before[1].position.values[0] - 10.).abs() < 1e-6);
    let mut files = MemorySources::default();
    files.insert("main.sanim","mut offset=Vec2(0pt,0pt)\nsig place=offset\nlet child=Circle(id=\"child\",position=place)\nadd(Row(id=\"row\",children=[child]))");
    let error = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert!(error.message.contains("flow layout controls"), "{error}");
    assert!(error.message.contains("position"), "{error}");
}

#[test]
fn constructor_properties_lift_direct_cells_and_pure_helper_captures() {
    use sanim_core::{Geometry, Number};
    let p = run(
        "mut radius=10pt\nsig size=radius*2\nfn get_size():Length{return size}\nlet direct=Circle(radius=radius)\nlet wrapped=Circle(radius=get_size())\nadd(direct)\nadd(wrapped)\nbackground{animate(radius,to=20pt,duration=2s)}\nwait(2s)\n",
    );
    let elements = p.animation.sample(Fraction::from(1)).unwrap();
    let circle_radius = |name: &str| {
        let element = elements.iter().find(|element| element.id == name).unwrap();
        let Geometry::Circle { radius } = element.geometry.as_ref() else {
            panic!("circle geometry")
        };
        *radius
    };
    assert_eq!(circle_radius("direct"), Fraction::from(15));
    assert_eq!(circle_radius("wrapped"), Fraction::from(30));
    assert!(matches!(p.bindings["radius"],Value::Number(Number{value,..}) if value==20.into()));
}

#[test]
fn imported_named_constructor_ids_are_qualified_but_root_ids_are_plain() {
    let mut files = MemorySources::default();
    files.insert("a.sanim", "let circle=Circle()");
    files.insert("b.sanim", "let circle=Circle()");
    files.insert("main.sanim","import \"a.sanim\" as a\nimport \"b.sanim\" as b\nlet circle=Circle()\nadd(a.circle)\nadd(b.circle)\nadd(circle)\n");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let ids = prepared
        .animation
        .layers
        .iter()
        .map(|layer| layer.element.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["a.sanim::circle", "b.sanim::circle", "circle"]);
}

#[test]
fn element_dimension_getters_return_lengths_instead_of_colors() {
    let prepared = run(
        "let circle=Circle(radius=12pt)\nlet rect=Rect(width=30pt,height=40pt,radius=4pt)\nlet circle_radius=circle.radius\nlet rect_width=rect.width\nlet rect_height=rect.height\nlet rect_radius=rect.radius\n",
    );
    for (name, expected) in [
        ("circle_radius", 12),
        ("rect_width", 30),
        ("rect_height", 40),
        ("rect_radius", 4),
    ] {
        let actual = prepared.bindings[name].number().unwrap();
        assert_eq!(actual.unit, sanim_core::Unit::Length);
        assert_eq!(actual.value, Fraction::from(expected));
    }
}

#[test]
fn background_assignments_and_helper_writes_do_not_leak_into_the_main_present() {
    for body in ["count=1", "let ignored=set_later()"] {
        let prepared = run(&format!(
            "mut count=0\nfn set_later():Number{{count=1;return 0}}\nbackground{{wait(2s);{body}}}\nlet before=count\nwait(1s)\nlet middle=count\nwait(1s)\nlet after=count\n"
        ));
        assert_eq!(number(&prepared, "before"), Fraction::from(0));
        assert_eq!(number(&prepared, "middle"), Fraction::from(0));
        assert_eq!(number(&prepared, "after"), Fraction::from(1));
        let source = format!(
            "mut count=0\nfn set_later():Number{{count=1;return 0}}\nbackground{{wait(2s);{body}}}\nwait(1s)\nwait(count*10s)\n"
        );
        let mut files = MemorySources::default();
        files.insert("main.sanim", &source);
        let planned = Engine::default()
            .duration("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        let full = run(&source);
        assert_eq!(planned.duration, Fraction::from(2));
        assert_eq!(planned.duration, full.animation.duration);
    }
}

#[test]
fn property_getters_follow_the_current_cursor_and_temporal_signals_refresh() {
    let prepared = run(
        "mut extent=20pt\nlet rect=Rect(width=extent,height=40pt)\nbackground{animate(extent,to=40pt,duration=2s)}\nwait(2s)\nlet current_width=rect.width\nsig angle=rect.rotation\nsig doubled=angle*2\nlet before=doubled\nRotateTo(rect, 90deg, duration=1s, ease=\"linear\")\nlet after=doubled\n",
    );
    assert_eq!(number(&prepared, "current_width"), Fraction::from(40));
    assert_eq!(number(&prepared, "before"), Fraction::from(0));
    assert_eq!(number(&prepared, "after"), Fraction::from(180));
}

#[test]
fn structural_group_children_allow_independently_retained_child_properties() {
    let prepared = run(
        "mut radius=10pt\nlet group=Group(children=[Circle(id=\"child\",radius=radius)])\nadd(group)\nbackground{animate(radius,to=20pt,duration=2s)}\nwait(2s)\n",
    );
    let elements = prepared.animation.sample(Fraction::from(1)).unwrap();
    let sanim_core::Geometry::Group { children } = elements[0].geometry.as_ref() else {
        panic!("group geometry")
    };
    let sanim_core::Geometry::Circle { radius } = children[0].geometry.as_ref() else {
        panic!("circle geometry")
    };
    assert_eq!(*radius, Fraction::from(15));
}

#[test]
fn unannotated_position_signals_refresh_after_ordinary_motion() {
    let prepared = run(
        "let rect=Rect()\nsig point=rect.position\nlet before=point\nMoveTo(rect, Vec2(20pt,0pt), duration=1s, ease=\"linear\")\nlet after=point\n",
    );
    assert_eq!(prepared.bindings["before"].vec2().unwrap().values, [0., 0.]);
    assert_eq!(prepared.bindings["after"].vec2().unwrap().values, [20., 0.]);
}

#[test]
fn retaining_an_ordinary_motion_getter_fails_instead_of_freezing() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","let first=Rect()\nsig point=first.position\nlet second=Circle(position=point)\nMoveTo(first, Vec2(20pt,0pt), duration=1s, ease=\"linear\")");
    let error = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.code, "S140");
    assert!(error.message.contains("same mut input"), "{error}");
}

#[test]
fn reactive_shape_factory_arguments_fail_clearly_instead_of_losing_provenance() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","mut size=10pt\nfn disk(radius:Length):Element{return Circle(radius=radius)}\nlet shape=disk(size)");
    let error = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(error.code, "S140");
    assert!(error.message.contains("snapshot"), "{error}");
}

#[test]
fn future_same_value_background_writes_remain_explicit_events() {
    let source = "mut count=0\nbackground{wait(2s);count=0}\ncount=1\nwait(2s)\nlet after=count\nwait(count*10s)\n";
    let prepared = run(source);
    assert_eq!(number(&prepared, "after"), Fraction::from(0));
    assert_eq!(prepared.animation.duration, Fraction::from(2));
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    assert_eq!(
        Engine::default()
            .duration("main.sanim", &files, &BTreeMap::new())
            .unwrap()
            .duration,
        prepared.animation.duration
    );
}

#[test]
fn background_list_writes_commit_at_their_timeline_point_including_helper_writes() {
    for body in ["rows=[1]", "let ignored=replace_rows()"] {
        let mut files = MemorySources::default();
        files.insert("main.sanim",format!("mut rows:List<Number>=[]\nfn replace_rows():Number{{rows=[1];return 0}}\nbackground{{wait(1s);{body}}}\nlet before=len(rows)\nwait(1s)\nlet after=len(rows)"));
        let prepared = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert_eq!(number(&prepared, "before"), Fraction::from(0));
        assert_eq!(number(&prepared, "after"), Fraction::from(1));
    }
}

#[test]
fn retained_record_xy_fields_and_imported_scalar_x_are_not_vector_components() {
    let mut files = MemorySources::default();
    files.insert("theme.sanim", "mut x=10pt");
    files.insert("main.sanim","import \"theme.sanim\" as theme\nlet point={x=10pt,y=0pt}\nmut offset=0pt\nlet record_shape=Circle(position=Vec2(point.x+offset,point.y))\nlet imported_shape=Circle(position=Vec2(theme.x,0pt))\nadd(record_shape)\nadd(imported_shape)\nbackground{animate(offset,to=20pt,duration=2s)}\nwait(2s)\n");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let elements = prepared.animation.sample(Fraction::from(1)).unwrap();
    assert_eq!(
        elements
            .iter()
            .find(|element| element.id == "record_shape")
            .unwrap()
            .position
            .values,
        [20., 0.]
    );
    assert_eq!(
        elements
            .iter()
            .find(|element| element.id == "imported_shape")
            .unwrap()
            .position
            .values,
        [10., 0.]
    );
}

#[test]
fn literal_children_reject_reactive_elements_and_topology_factories() {
    for child in ["child", "make()"] {
        let mut files = MemorySources::default();
        files.insert("main.sanim",format!("mut choose_circle=true\nfn make():Element{{if choose_circle{{return Circle()}}else{{return Rect()}}}}\nsig child=make()\nlet group=Group(children=[{child}])\nadd(group)\nchoose_circle=false\nwait(1s)"));
        let error = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err();
        assert_eq!(error.code, "S140");
        assert!(error.message.contains("topology"), "{error}");
    }
    let prepared = run(
        "mut choose_circle=true\nfn make():Element{if choose_circle{return Circle()}else{return Rect()}}\nlet snapshot=make()\nlet group=Group(children=[snapshot])\nadd(group)\nchoose_circle=false\nwait(1s)",
    );
    assert_eq!(prepared.animation.layers.len(), 1);
}

#[test]
fn background_pure_helper_local_data_mutation_remains_private_scratch() {
    let source = "mut output=0\nfn calculate():Number{mut rows:List<Number>=[];rows=[1,2];return len(rows)}\nbackground{wait(1s);output=calculate()}\nlet before=output\nwait(1s)\nlet after=output\n";
    let prepared = run(source);
    assert_eq!(number(&prepared, "before"), Fraction::from(0));
    assert_eq!(number(&prepared, "after"), Fraction::from(2));
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    assert_eq!(
        Engine::default()
            .duration("main.sanim", &files, &BTreeMap::new())
            .unwrap()
            .duration,
        prepared.animation.duration
    );
}

#[test]
fn nested_background_writes_to_parent_local_cells_keep_their_timeline() {
    let source = "mut before=-1\nmut middle=-1\nmut after=-1\nbackground{mut local=0;background{wait(2s);local=1};before=local;wait(1s);middle=local;wait(1s);after=local}\nwait(2s)\n";
    let prepared = run(source);
    assert_eq!(number(&prepared, "before"), Fraction::from(0));
    assert_eq!(number(&prepared, "middle"), Fraction::from(0));
    assert_eq!(number(&prepared, "after"), Fraction::from(1));
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    assert_eq!(
        Engine::default()
            .duration("main.sanim", &files, &BTreeMap::new())
            .unwrap()
            .duration,
        prepared.animation.duration
    );
}
