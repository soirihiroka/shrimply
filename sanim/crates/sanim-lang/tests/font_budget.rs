use sanim_lang::{Engine, MemorySources};

const FONT: &[u8] = include_bytes!("../../../examples/assets/DejaVuSansCondensed-subset.ttf");
const PADDING: usize = 1024 * 1024;
const LABELS: usize = 32;

fn files() -> (MemorySources, usize) {
    let mut files = MemorySources::default();
    let mut padded = FONT.to_vec();
    padded.resize(padded.len() + PADDING, 0);
    let bytes = padded.len();
    files.insert("font.ttf", &padded);
    padded.push(0);
    files.insert("other.ttf", padded);
    (files, bytes)
}
fn engine(font_bytes: usize) -> Engine {
    let mut engine = Engine::default();
    engine.limits.data.max_bytes = font_bytes + 256 * 1024;
    engine
}

#[test]
fn separate_labels_share_one_font_charge_and_restore_that_budget_with_the_prefix() {
    let (mut files, font_bytes) = files();
    let prefix = format!(
        "let typeface=read<Font>(\"font.ttf\")\n{}",
        (0..LABELS)
            .map(|index| format!("let label_{index}=Label(\"x\",font=typeface)\n"))
            .collect::<String>()
    );
    files.insert("main.sanim", format!("{prefix}wait(1s)\n"));
    let mut engine = engine(font_bytes);
    let first = engine
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    assert!(first.bindings.contains_key("label_31"));
    files.insert(
        "main.sanim",
        format!("{prefix}let extra=Label(\"x\",font=typeface)\nwait(2s)\n"),
    );
    let reloaded = engine
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    assert_eq!(reloaded.statistics.restored_statements, LABELS as u64 + 1);
    assert!(reloaded.bindings.contains_key("extra"));
}

#[test]
fn persistent_list_appends_charge_shared_fonts_once_and_only_copy_the_changed_path() {
    let (mut files, font_bytes) = files();
    files.insert("main.sanim", format!("let typeface=read<Font>(\"font.ttf\")\nmut labels:List<Element>=[]\nrepeat {LABELS} {{\nlabels=append(labels,Label(\"x\",font=typeface))\n}}\nwait(1s)\n"));
    let mut engine = engine(font_bytes);
    // Retained path copies have their own cost; this still cannot hide 32 copies of the TTF.
    engine.limits.data.max_bytes = font_bytes + 2 * 1024 * 1024;
    let prepared = engine
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    assert_eq!(prepared.bindings["labels"].list().unwrap().len(), LABELS);
}

#[test]
fn distinct_font_payloads_still_pay_their_full_retained_cost() {
    let (mut files, font_bytes) = files();
    files.insert(
        "main.sanim",
        "let first=read<Font>(\"font.ttf\")\nlet second=read<Font>(\"other.ttf\")\nwait(1s)\n",
    );
    let error = engine(font_bytes)
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert_eq!(error.code, "S303");
    assert!(error.message.contains("byte budget"), "{error}");
}
