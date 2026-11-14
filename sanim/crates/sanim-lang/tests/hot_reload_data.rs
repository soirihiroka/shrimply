use sanim_core::{Color, Fraction, Value};
use sanim_lang::{Engine, MemorySources, Preparation, Reload, Session};
use std::collections::BTreeMap;

fn program(color: &str, revision: usize) -> String {
    format!(
        "let tint=Color(\"{color}\")\ntype Row={{value:Number}}\ntype Total={{total:Number}}\nlet rows=read<List<Row>>(\"rows.json\")\nlet totals=query<Total>(\"SELECT sum(value) AS total FROM rows\",tables={{rows=rows}})\nfor total in totals{{assert(total.total>=0)}}\nlet marker=Circle(fill=tint)\nadd(marker)\nwait(3s)\n# revision {revision}"
    )
}
fn total(prepared: &sanim_lang::Prepared) -> Fraction {
    let Value::Record(row) = prepared.bindings["totals"].list().unwrap().front().unwrap() else {
        panic!("typed SQL row")
    };
    row["total"].number().unwrap().value
}

#[test]
fn unrelated_visual_edits_reuse_data_and_sql_in_both_preparation_modes() {
    const ROWS: usize = 10_000;
    const EDITS: usize = 32;
    let data = format!(
        "[{}]",
        (0..ROWS)
            .map(|value| format!("{{\"value\":{value}}}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    for preparation in [Preparation::Full, Preparation::Demand] {
        let mut files = MemorySources::default();
        files.insert("rows.json", &data);
        files.insert("main.sanim", program("blue3", 0));
        let mut engine = Engine::default();
        engine.preparation = preparation;
        let cold = engine
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert_eq!(
            (
                cold.statistics.decoded_assets,
                cold.statistics.executed_queries
            ),
            (1, 1)
        );
        let expected = Fraction::from(ROWS * (ROWS - 1) / 2);
        assert_eq!(total(&cold), expected);
        for revision in 1..=EDITS {
            let color = if revision % 2 == 0 {
                "blue3"
            } else {
                "orange3"
            };
            files.insert("main.sanim", program(color, revision));
            let hot = engine
                .prepare("main.sanim", &files, &BTreeMap::new())
                .unwrap();
            assert_eq!(
                (
                    hot.statistics.decoded_assets,
                    hot.statistics.executed_queries,
                    hot.statistics.sql_instructions
                ),
                (0, 0, 0)
            );
            assert!(
                hot.statistics.operations < 100,
                "hot edit must not replay the data work: {:?}",
                hot.statistics
            );
            assert_eq!(total(&hot), expected);
            assert_eq!(
                hot.animation.sample(Fraction::from(0)).unwrap()[0].fill,
                sanim_core::Paint::Solid(Color::parse(color).unwrap())
            );
        }
        assert_eq!(
            total(&cold),
            expected,
            "old immutable prepared roots must survive later edits"
        );
    }
}

#[test]
fn changed_data_invalidates_query_keys_and_failed_data_preserves_last_good() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", program("blue3", 0));
    files.insert("rows.json", "[{\"value\":1}]");
    let mut session = Session::new("main.sanim");
    assert!(matches!(session.poll(&files), Reload::Updated { .. }));
    assert_eq!(total(session.current().unwrap()), Fraction::from(1));
    files.insert("rows.json", "[{\"value\":7},{\"value\":8}]");
    let Reload::Updated { prepared, .. } = session.poll(&files) else {
        panic!("data revision")
    };
    assert_eq!(total(&prepared), Fraction::from(15));
    assert_eq!(
        (
            prepared.statistics.decoded_assets,
            prepared.statistics.executed_queries
        ),
        (1, 1)
    );
    files.insert("rows.json", "[{\"value\":\"bad\"}]");
    assert!(matches!(session.poll(&files), Reload::Failed { .. }));
    assert_eq!(total(session.current().unwrap()), Fraction::from(15));
    files.insert("rows.json", "[{\"value\":3}]");
    assert!(matches!(session.poll(&files), Reload::Updated { .. }));
    assert_eq!(total(session.current().unwrap()), Fraction::from(3));
    assert_eq!(total(&prepared), Fraction::from(15));
}
