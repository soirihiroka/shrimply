use sanim_core::{
    Fraction, TimelineEffect,
    duration::{
        DurationExpr as Expr, DurationSymbol, MAX_DURATION_ALTERNATIVES, MAX_DURATION_TERMS,
    },
};

fn number(n: u64) -> Expr {
    Expr::constant(Fraction::from(n)).unwrap()
}
fn parameter(name: &str) -> Expr {
    Expr::parameter(name).unwrap()
}

#[test]
fn exact_parameter_and_length_formulas_substitute_without_execution() {
    let effect = TimelineEffect::Fixed(Fraction::new(1u64, 4u64)).repeated(parameter("n"));
    assert!(effect.to_string().contains("n"));
    assert_eq!(effect.resolve(&mut |_| None).unwrap(), None);
    assert_eq!(
        effect
            .resolve(
                &mut |s| (s == &DurationSymbol::Parameter("n".into())).then_some(Fraction::from(8))
            )
            .unwrap(),
        Some(vec![Fraction::from(2)])
    );
    let list = TimelineEffect::Fixed(Fraction::new(1u64, 10u64))
        .repeated(Expr::length("samples").unwrap());
    assert!(list.to_string().contains("len(samples)"));
    assert_eq!(
        list.resolve(&mut |s| matches!(s, DurationSymbol::Length(_)).then_some(Fraction::from(30)))
            .unwrap(),
        Some(vec![Fraction::from(3)])
    );
    let symbolic = effect
        .substitute(&mut |_| Expr::product([parameter("rows"), number(2)]))
        .unwrap();
    assert_eq!(
        symbolic.resolve(&mut |_| Some(Fraction::from(4))).unwrap(),
        Some(vec![Fraction::from(2)])
    );
}

#[test]
fn composition_is_normalized_and_branch_alternatives_are_not_maximums() {
    assert_eq!(
        Expr::sum([number(2), parameter("n"), number(3)]),
        Expr::sum([parameter("n"), number(5)])
    );
    assert_eq!(
        Expr::product([parameter("n"), number(2), number(3)]),
        Expr::product([number(6), parameter("n")])
    );
    let branch = TimelineEffect::alternatives([
        TimelineEffect::Fixed(Fraction::from(2)),
        TimelineEffect::Fixed(Fraction::from(1)),
        TimelineEffect::Fixed(Fraction::from(2)),
    ])
    .unwrap();
    assert_eq!(branch.to_string(), "1s | 2s");
    assert_eq!(
        branch
            .clone()
            .then(TimelineEffect::Fixed(Fraction::from(1)))
            .unwrap()
            .to_string(),
        "2s | 3s"
    );
    assert_eq!(
        branch
            .clone()
            .repeated(number(2))
            .resolve(&mut |_| None)
            .unwrap(),
        Some(vec![
            Fraction::from(2),
            Fraction::from(3),
            Fraction::from(4)
        ])
    );
    assert_eq!(
        branch.clone().repeated(parameter("n")),
        TimelineEffect::Unknown
    );
    assert_eq!(
        branch
            .parallel(TimelineEffect::Fixed(Fraction::new(3u64, 2u64)))
            .resolve(&mut |_| None)
            .unwrap(),
        Some(vec![Fraction::new(3u64, 2u64), Fraction::from(2)])
    );
    assert_eq!(
        TimelineEffect::Fixed(Fraction::from(1)).parallel(TimelineEffect::Fixed(Fraction::from(2))),
        TimelineEffect::Fixed(Fraction::from(2))
    );
}

#[test]
fn resolved_duration_rejects_negative_nonfinite_and_two_hour_overflow() {
    let effect = TimelineEffect::Fixed(Fraction::from(1)).repeated(parameter("n"));
    assert!(
        effect
            .resolve(&mut |_| Some(Fraction::from(7201)))
            .unwrap_err()
            .contains("two-hour")
    );
    assert!(
        effect
            .resolve(&mut |_| Some(-Fraction::from(1)))
            .unwrap_err()
            .contains("nonnegative")
    );
    assert!(
        effect
            .resolve(&mut |_| Some(Fraction::infinity()))
            .unwrap_err()
            .contains("finite")
    );
    assert_eq!(
        effect.resolve(&mut |_| Some(Fraction::from(7200))).unwrap(),
        Some(vec![Fraction::from(7200)])
    );
}

#[test]
fn excessive_formulas_fall_back_without_unbounded_retained_trees() {
    assert!(Expr::sum((0..=MAX_DURATION_TERMS).map(|i| parameter(&format!("n{i}")))).is_none());
    assert_eq!(
        TimelineEffect::alternatives(
            (0..=MAX_DURATION_ALTERNATIVES)
                .map(|i| TimelineEffect::Fixed(Fraction::from(i as u64)))
        )
        .unwrap(),
        TimelineEffect::Unknown
    );
    let mut expr = parameter("n");
    for _ in 0..1000 {
        let Some(next) = Expr::product([
            expr.clone(),
            Expr::sum([parameter("m"), number(1)]).unwrap(),
        ]) else {
            break;
        };
        expr = next;
    }
    assert!(expr.complexity().unwrap() <= sanim_core::duration::MAX_DURATION_NODES);
    assert_eq!(
        TimelineEffect::Unknown.repeated(number(0)),
        TimelineEffect::default()
    );
}

#[test]
fn formulas_and_alternatives_round_trip_serialization() {
    let effect = TimelineEffect::alternatives([
        TimelineEffect::Fixed(Fraction::from(1)),
        TimelineEffect::Fixed(Fraction::new(1u64, 4u64)).repeated(parameter("n")),
    ])
    .unwrap();
    let decoded: TimelineEffect =
        serde_json::from_slice(&serde_json::to_vec(&effect).unwrap()).unwrap();
    assert_eq!(decoded, effect);
    assert_eq!(
        decoded.resolve(&mut |_| Some(Fraction::from(8))).unwrap(),
        Some(vec![Fraction::from(1), Fraction::from(2)])
    );
}

#[test]
#[ignore = "manual symbolic duration resolution performance measurement"]
fn symbolic_duration_resolution_benchmark() {
    use std::{hint::black_box, time::Instant};
    const ITERATIONS: usize = 2_000;
    let effect = TimelineEffect::Fixed(Fraction::new(1u64, 400u64)).repeated(parameter("n"));
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        black_box(effect.resolve(&mut |_| Some(Fraction::from(1000))).unwrap());
    }
    let symbolic = start.elapsed();
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let mut total = Fraction::from(0);
        for _ in 0..black_box(1000) {
            total = sanim_core::math::timeline_end(total, Fraction::new(1u64, 400u64)).unwrap();
        }
        black_box(total);
    }
    let repeated = start.elapsed();
    eprintln!(
        "{ITERATIONS} duration queries ×1000 repetitions: symbolic={symbolic:?}, repeated={repeated:?}"
    );
    assert!(
        symbolic * 4 < repeated,
        "symbolic duration resolution should avoid executing loop bodies"
    );
}

#[test]
fn duration_heap_accounting_shares_cloned_roots_but_charges_distinct_allocations() {
    use sanim_core::duration::DurationBudget;
    use std::sync::Arc;
    let symbol: Arc<str> = "shared_parameter_name".into();
    let effect = TimelineEffect::from_expr(Expr::parameter(symbol.clone()).unwrap());
    let mut budget = DurationBudget::default();
    let mut remaining = 1024 * 1024;
    budget.charge(&effect, &mut remaining).unwrap();
    let after_first = remaining;
    for _ in 0..10_000 {
        budget.charge(&effect.clone(), &mut remaining).unwrap();
    }
    assert_eq!(
        remaining, after_first,
        "shared formula allocations must be charged once per publication"
    );
    let another = TimelineEffect::from_expr(Expr::parameter(symbol).unwrap());
    budget.charge(&another, &mut remaining).unwrap();
    let shared_name_cost = after_first - remaining;
    let before = remaining;
    let distinct = TimelineEffect::from_expr(Expr::parameter("shared_parameter_name").unwrap());
    budget.charge(&distinct, &mut remaining).unwrap();
    assert!(
        before - remaining > shared_name_cost,
        "separate symbol allocations need their own payload charge"
    );
    let mut tiny = 0;
    assert!(
        DurationBudget::default()
            .charge(&effect, &mut tiny)
            .unwrap_err()
            .contains("duration metadata budget")
    );
}

#[test]
fn type_budget_includes_duration_heaps_and_shares_them_across_signatures() {
    use sanim_core::{Signature, Type, duration::DurationBudget};
    use std::collections::{BTreeMap, BTreeSet};
    let effect = TimelineEffect::Fixed(Fraction::new(1u64, 4u64)).repeated(parameter("n"));
    let function = |duration| {
        Type::Function(Signature {
            duration,
            schedules: true,
            mutates: false,
            duration_parameters: BTreeSet::new(),
            generics: vec![],
            default_count: 0,
            parameters: vec![],
            result: Box::new(Type::Unit),
        })
    };
    let schema = |effect: TimelineEffect| {
        Type::Record(
            (0..256)
                .map(|i| (format!("f{i}"), function(effect.clone())))
                .collect::<BTreeMap<_, _>>(),
        )
    };
    let mut baseline = usize::MAX;
    sanim_core::type_budget::charge(&schema(TimelineEffect::default()), &mut baseline).unwrap();
    let mut actual = usize::MAX;
    sanim_core::type_budget::charge(&schema(effect.clone()), &mut actual).unwrap();
    let mut heap = usize::MAX;
    DurationBudget::default()
        .charge(&effect, &mut heap)
        .unwrap();
    assert_eq!(
        baseline - actual,
        usize::MAX - heap,
        "one duration DAG shared by256 signatures must be billed once within that type traversal"
    );
    let mut tiny = 2 * std::mem::size_of::<Type>();
    assert!(
        sanim_core::type_budget::charge(&function(effect), &mut tiny)
            .unwrap_err()
            .contains("duration metadata budget")
    );
}

#[test]
#[ignore = "manual shared duration metadata accounting performance measurement"]
fn duration_heap_accounting_benchmark() {
    use sanim_core::duration::DurationBudget;
    use std::{hint::black_box, time::Instant};
    const REFERENCES: usize = 10_000;
    let effect = TimelineEffect::Fixed(Fraction::new(1u64, 4u64)).repeated(parameter("n"));
    let mut samples = Vec::new();
    for _ in 0..7 {
        let mut budget = DurationBudget::default();
        let mut remaining = 1024 * 1024;
        budget.charge(&effect, &mut remaining).unwrap();
        let after_first = remaining;
        let start = Instant::now();
        for _ in 0..REFERENCES {
            budget.charge(black_box(&effect), &mut remaining).unwrap();
        }
        let shared = start.elapsed();
        assert_eq!(remaining, after_first);
        let start = Instant::now();
        for _ in 0..REFERENCES {
            DurationBudget::default()
                .charge(black_box(&effect), &mut (1024 * 1024))
                .unwrap();
        }
        samples.push((shared, start.elapsed()));
    }
    samples.sort_unstable_by_key(|sample| sample.0);
    let shared = samples[samples.len() / 2].0;
    samples.sort_unstable_by_key(|sample| sample.1);
    let fresh = samples[samples.len() / 2].1;
    eprintln!(
        "{REFERENCES} references to one duration DAG: shared-publication={shared:?}, independent-accounting={fresh:?}; repeated shared heap bytes=0"
    );
}
