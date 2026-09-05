// Ported from Rhubarb Lip Sync's phonetic mouth-animation pipeline.

use std::collections::BTreeMap;

use shrimply_pocketsphinx::Phone as SphinxPhone;

use crate::{MouthCue, MouthShape, timeline::Timeline};

const MAX_ANTICIPATION_CENTISECONDS: i64 = 20;
const MIN_SHAPE_CENTISECONDS: i64 = 7;
const MIN_MOUTH_STATE_CENTISECONDS: i64 = 8;
const MAX_STATE_EXTENSION_CENTISECONDS: i64 = 6;
const MIN_TWEEN_CENTISECONDS: i64 = 4;
const MAX_TWEEN_CENTISECONDS: i64 = 8;
const MIN_STATIC_CENTISECONDS: i64 = 75;
const MIN_STATIC_SYLLABLES: usize = 3;
const MAX_STATIC_REPLACEMENTS: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phone {
    Sphinx(SphinxPhone),
    Schwa,
    Noise,
}

impl Phone {
    pub fn from_sphinx(phone: SphinxPhone, duration: i64) -> Self {
        match phone {
            SphinxPhone::AH if duration < 6 => Self::Schwa,
            SphinxPhone::Silence | SphinxPhone::UhFiller | SphinxPhone::UmFiller => Self::Noise,
            phone => Self::Sphinx(phone),
        }
    }

    fn is_vowel(self) -> bool {
        matches!(
            self,
            Self::Schwa
                | Self::Sphinx(
                    SphinxPhone::AA
                        | SphinxPhone::AE
                        | SphinxPhone::AH
                        | SphinxPhone::AO
                        | SphinxPhone::AW
                        | SphinxPhone::AY
                        | SphinxPhone::EH
                        | SphinxPhone::ER
                        | SphinxPhone::EY
                        | SphinxPhone::IH
                        | SphinxPhone::IY
                        | SphinxPhone::OW
                        | SphinxPhone::OY
                        | SphinxPhone::UH
                        | SphinxPhone::UW
                )
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TimedPhone {
    pub start: i64,
    pub end: i64,
    pub phone: Phone,
}

impl TimedPhone {
    pub const fn noise(start: i64, end: i64) -> Self {
        Self {
            start,
            end,
            phone: Phone::Noise,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShapeSet(u16);

impl ShapeSet {
    const fn one(shape: MouthShape) -> Self {
        Self(1 << shape as u8)
    }

    fn from_slice(shapes: &[MouthShape]) -> Self {
        Self(
            shapes
                .iter()
                .fold(0, |bits, shape| bits | (1 << *shape as u8)),
        )
    }

    const fn contains(self, shape: MouthShape) -> bool {
        self.0 & (1 << shape as u8) != 0
    }

    const fn len(self) -> u32 {
        self.0.count_ones()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShapeRule {
    shapes: ShapeSet,
    phone: Option<Phone>,
    phone_start: i64,
    phone_end: i64,
}

impl ShapeRule {
    const fn silence() -> Self {
        Self {
            shapes: ShapeSet::one(MouthShape::X),
            phone: None,
            phone_start: 0,
            phone_end: 0,
        }
    }
}

pub(crate) fn animate(phones: &[TimedPhone], duration: i64) -> Vec<MouthCue> {
    let rules = shape_rules(phones, duration);
    let animation = avoid_static_segments(&rules);
    animation
        .spans()
        .iter()
        .map(|span| MouthCue {
            start_centiseconds: span.start,
            end_centiseconds: span.end,
            shape: span.value,
        })
        .collect()
}

fn shape_rules(phones: &[TimedPhone], duration: i64) -> Timeline<ShapeRule> {
    // The sequence number keeps adjacent instances of the same phone separate,
    // as Rhubarb's non-joining source phone timeline does.
    let mut phone_timeline = Timeline::new_unjoined(0, duration, (None, 0_usize));
    for (index, phone) in phones.iter().enumerate() {
        phone_timeline.set(phone.start, phone.end, (Some(phone.phone), index + 1));
    }

    let mut rules = Timeline::new_unjoined(0, duration, ShapeRule::silence());
    let mut previous_duration = 0;
    for timed_phone in phone_timeline.spans() {
        let phone_duration = timed_phone.duration();
        if let Some(phone) = timed_phone.value.0 {
            for (start, end, shapes) in shape_sets(phone, phone_duration, previous_duration) {
                rules.set(
                    timed_phone.start + start,
                    timed_phone.start + end,
                    ShapeRule {
                        shapes,
                        phone: Some(phone),
                        phone_start: timed_phone.start,
                        phone_end: timed_phone.end,
                    },
                );
            }
        }
        previous_duration = phone_duration;
    }
    rules
}

fn shape_sets(phone: Phone, duration: i64, previous_duration: i64) -> Vec<(i64, i64, ShapeSet)> {
    use MouthShape::{A, B, C, D, E, F, G, H, X};
    use SphinxPhone as P;

    let one = |shapes: &[MouthShape]| vec![(0, duration, ShapeSet::from_slice(shapes))];
    let diphthong = |first: &[MouthShape], second: &[MouthShape]| {
        let first_duration = duration * 6 / 10;
        vec![
            (0, first_duration, ShapeSet::from_slice(first)),
            (first_duration, duration, ShapeSet::from_slice(second)),
        ]
    };
    let plosive = |first: &[MouthShape], second: &[MouthShape]| {
        let occlusion = (previous_duration / 2).clamp(4, 12);
        vec![
            (-occlusion, 0, ShapeSet::from_slice(first)),
            (0, duration, ShapeSet::from_slice(second)),
        ]
    };
    let any = &[A, B, C, D, E, F, G, H, X];
    let any_open = &[B, C, D, E, F, G, H];

    match phone {
        Phone::Schwa => one(&[B, C]),
        Phone::Noise => one(&[B]),
        Phone::Sphinx(phone) => match phone {
            P::AO => one(&[E]),
            P::AA => one(&[D]),
            P::IY | P::IH => one(&[B]),
            P::UW | P::UH => one(&[F]),
            P::EH | P::AE => one(&[C]),
            P::AH => one(if duration < 20 { &[C] } else { &[D] }),
            P::EY => diphthong(&[C], &[B]),
            P::AY => diphthong(if duration < 20 { &[C] } else { &[D] }, &[B]),
            P::OW => diphthong(&[E], &[F]),
            P::AW => diphthong(if duration < 30 { &[C] } else { &[D] }, &[E]),
            P::OY => diphthong(&[E], &[B]),
            P::ER if duration < 7 => one(&[B, C]),
            P::ER => one(&[E]),
            P::P | P::B => plosive(&[A], any),
            P::T | P::D => plosive(&[B, F], any_open),
            P::K | P::G => plosive(&[B, C, E, F, H], any_open),
            P::CH | P::JH | P::TH | P::DH | P::S | P::Z | P::SH | P::ZH => one(&[B, F]),
            P::F | P::V => one(&[G]),
            P::HH => one(any),
            P::M => one(&[A]),
            P::N => one(&[B, C, F, H]),
            P::NG => one(&[B, C, E, F]),
            P::L if duration < 20 => one(&[B, E, F, H]),
            P::L => one(&[H]),
            P::R => one(&[B, E, F]),
            P::Y => one(&[B, C, F]),
            P::W => one(&[F]),
            P::Breath | P::Cough | P::Smack => one(&[C]),
            P::Noise | P::Silence | P::UhFiller | P::UmFiller => one(&[B]),
        },
    }
}

fn animate_rules(rules: &Timeline<ShapeRule>) -> Timeline<MouthShape> {
    let rough = animate_rough(rules);
    let timed = optimize_timing(&rough);
    let pauses = animate_pauses(&timed);
    insert_tweens(&pauses)
}

fn animate_rough(rules: &Timeline<ShapeRule>) -> Timeline<MouthShape> {
    let mut animation = Timeline::new(rules.start(), rules.end(), MouthShape::X);
    let mut reference = MouthShape::X;
    let mut last_anticipated_start = -1;

    for (index, timed_rule) in rules.spans().iter().enumerate() {
        let rule = timed_rule.value;
        let shape = closest_shape(reference, rule.shapes);
        animation.set(timed_rule.start, timed_rule.end, shape);
        let anticipate = rule.phone.is_some_and(Phone::is_vowel) && rule.shapes.len() == 1;
        if anticipate {
            reference = shape;
            for preceding in rules.spans()[..index].iter().rev() {
                if preceding.start == last_anticipated_start
                    || timed_rule.start - preceding.start > MAX_ANTICIPATION_CENTISECONDS
                {
                    break;
                }
                let anticipating = closest_shape(reference, preceding.value.shapes);
                animation.set(preceding.start, preceding.end, anticipating);
                if basic_shape(anticipating) != basic_shape(shape) {
                    break;
                }
                reference = anticipating;
            }
            last_anticipated_start = timed_rule.start;
        }
        reference = if anticipate { shape } else { relax(shape) };
    }
    animation
}

const fn basic_shape(shape: MouthShape) -> MouthShape {
    use MouthShape::*;
    match shape {
        A | G | X => A,
        B => B,
        C | H => C,
        D => D,
        E => E,
        F => F,
    }
}

const fn relax(shape: MouthShape) -> MouthShape {
    use MouthShape::*;
    match shape {
        A => A,
        B | F | H => B,
        C | D | E => C,
        G | X => X,
    }
}

fn closest_shape(reference: MouthShape, shapes: ShapeSet) -> MouthShape {
    use MouthShape::*;
    const EFFORT: [[MouthShape; 9]; 9] = [
        [A, X, G, B, C, H, E, D, F],
        [B, G, A, X, C, H, E, D, F],
        [C, H, B, G, D, A, X, E, F],
        [D, C, H, B, G, A, X, E, F],
        [E, C, H, B, G, A, X, D, F],
        [F, B, G, A, X, C, H, E, D],
        [G, A, B, C, H, X, E, D, F],
        [H, C, B, G, D, A, X, E, F],
        [X, A, G, B, C, H, E, D, F],
    ];
    *EFFORT[reference as usize]
        .iter()
        .find(|shape| shapes.contains(**shape))
        .expect("Rhubarb shape rule contains no mouth shape")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MouthState {
    Idle,
    Closed,
    Open,
}

fn optimize_timing(animation: &Timeline<MouthShape>) -> Timeline<MouthShape> {
    let mut states = Timeline::new(animation.start(), animation.end(), MouthState::Idle);
    for shape in animation.spans() {
        states.set(
            shape.start,
            shape.end,
            match shape.value {
                MouthShape::X => MouthState::Idle,
                MouthShape::A => MouthState::Closed,
                _ => MouthState::Open,
            },
        );
    }

    let mut result = Timeline::new(animation.start(), animation.end(), MouthShape::X);
    let mut result_start = animation.end();
    let mut index = states.spans().len();
    while index > 0 {
        index -= 1;
        let segment = &states.spans()[index];
        if segment.value == MouthState::Idle {
            continue;
        }
        result_start = result_start.min(segment.end);
        if result_start - segment.start >= MIN_MOUTH_STATE_CENTISECONDS {
            copy_retimed(
                &mut result,
                animation,
                segment.start,
                segment.end,
                segment.start,
                result_start,
            );
            result_start = segment.start;
            continue;
        }

        let right = index;
        let mut left = index;
        while left > 0 {
            let candidate = &states.spans()[left - 1];
            if candidate.value == MouthState::Idle
                || candidate.duration() >= MIN_MOUTH_STATE_CENTISECONDS
            {
                break;
            }
            left -= 1;
        }
        let count = right - left + 1;
        let desired = MIN_MOUTH_STATE_CENTISECONDS * count as i64;
        let current = states.spans()[right].end - states.spans()[left].start;
        let available = if left > 0 {
            states.spans()[left - 1].duration() - 1
        } else {
            0
        };
        let extension = (desired - current)
            .min(available)
            .clamp(0, MAX_STATE_EXTENSION_CENTISECONDS);
        let target_start = states.spans()[left].start - extension;
        for source_index in (left..=right).rev() {
            let remaining = source_index - left + 1;
            let segment_duration = (result_start - target_start) / remaining as i64;
            let segment_target_start = result_start - segment_duration;
            let source = &states.spans()[source_index];
            copy_retimed(
                &mut result,
                animation,
                source.start,
                source.end,
                segment_target_start,
                result_start,
            );
            result_start = segment_target_start;
        }
        index = left;
    }
    result
}

fn copy_retimed(
    result: &mut Timeline<MouthShape>,
    animation: &Timeline<MouthShape>,
    source_start: i64,
    source_end: i64,
    target_start: i64,
    target_end: i64,
) {
    let source = animation.clipped_spans(source_start, source_end);
    if source.is_empty() {
        return;
    }
    let mut write_position = target_end;
    while write_position > target_start {
        let reduction = next_reduction(
            &source,
            source_start,
            source_end,
            target_start,
            target_end,
            write_position,
        );
        let mut shape_start = reduction.first().expect("non-empty reduction").start;
        if shape_start <= source_start {
            shape_start = target_start.min(shape_start);
        }
        shape_start = shape_start.max(target_start);
        let shape_end = reduction
            .last()
            .expect("non-empty reduction")
            .end
            .min(write_position);
        result.set(shape_start, shape_end, representative_shape(&reduction));
        write_position = shape_start;
    }
}

fn next_reduction(
    source: &[crate::timeline::Span<MouthShape>],
    source_start: i64,
    source_end: i64,
    target_start: i64,
    target_end: i64,
    write_position: i64,
) -> Vec<crate::timeline::Span<MouthShape>> {
    let minimal = reduction_for_range(
        source,
        minimal_candidate_range(
            source,
            source_start,
            source_end,
            target_start,
            target_end,
            write_position,
        ),
    );
    let extended = reduction_for_range(
        source,
        (
            minimal.first().expect("non-empty reduction").start,
            minimal.last().expect("non-empty reduction").end,
        ),
    );
    let next = reduction_for_range(
        source,
        minimal_candidate_range(
            source,
            source_start,
            source_end,
            target_start,
            target_end,
            minimal.first().expect("non-empty reduction").start,
        ),
    );
    let min_shape = representative_shape(&minimal);
    let extended_shape = representative_shape(&extended);
    if min_shape == extended_shape
        || (extended_shape != min_shape && extended_shape != representative_shape(&next))
    {
        extended
    } else {
        minimal
    }
}

fn minimal_candidate_range(
    source: &[crate::timeline::Span<MouthShape>],
    source_start: i64,
    source_end: i64,
    target_start: i64,
    target_end: i64,
    write_position: i64,
) -> (i64, i64) {
    let remaining = write_position - target_start;
    let duration = if remaining <= MIN_SHAPE_CENTISECONDS || remaining >= 2 * MIN_SHAPE_CENTISECONDS
    {
        MIN_SHAPE_CENTISECONDS
    } else {
        remaining / 2
    };
    let mut start = write_position - duration;
    let mut end = write_position;
    if write_position == target_end {
        end = end.max(source_end);
    }
    if start >= source_end {
        start = source.last().expect("source is non-empty").start;
    }
    if end <= source_start {
        end = source.first().expect("source is non-empty").end;
    }
    (start, end)
}

fn reduction_for_range(
    source: &[crate::timeline::Span<MouthShape>],
    range: (i64, i64),
) -> Vec<crate::timeline::Span<MouthShape>> {
    source
        .iter()
        .filter_map(|span| {
            let start = span.start.max(range.0);
            let end = span.end.min(range.1);
            (start < end).then_some(crate::timeline::Span {
                start,
                end,
                value: span.value,
            })
        })
        .collect()
}

fn representative_shape(spans: &[crate::timeline::Span<MouthShape>]) -> MouthShape {
    let mut weights = BTreeMap::new();
    for span in spans {
        *weights.entry(span.value).or_insert(0) += span.duration();
    }
    let mut candidates = weights.iter();
    let (&mut_best, &mut_best_weight) = candidates
        .next()
        .expect("cannot reduce an empty shape range");
    let mut best = mut_best;
    let mut best_weight = mut_best_weight;
    for (&shape, &weight) in candidates {
        if weight > best_weight {
            best = shape;
            best_weight = weight;
        }
    }
    if best == MouthShape::C && weights.contains_key(&MouthShape::D) {
        MouthShape::D
    } else {
        best
    }
}

fn animate_pauses(animation: &Timeline<MouthShape>) -> Timeline<MouthShape> {
    let mut result = animation.clone();
    for window in animation.spans().windows(3) {
        let [previous, pause, next] = window else {
            unreachable!()
        };
        if pause.value == MouthShape::X {
            result.set(
                pause.start,
                pause.end,
                pause_shape(previous.value, next.value, pause.duration()),
            );
        }
    }
    result
}

fn pause_shape(previous: MouthShape, next: MouthShape, duration: i64) -> MouthShape {
    if duration < 12 {
        return previous;
    }
    if duration <= 35 {
        let mut current = previous;
        loop {
            let relaxed = relax(current);
            if relaxed != next {
                return relaxed;
            }
            if relaxed == current {
                break;
            }
            current = relaxed;
        }
    }
    MouthShape::X
}

#[derive(Clone, Copy)]
enum TweenTiming {
    Early,
    Centered,
    Late,
}

fn insert_tweens(animation: &Timeline<MouthShape>) -> Timeline<MouthShape> {
    let mut result = animation.clone();
    for pair in animation.spans().windows(2) {
        let [first, second] = pair else {
            unreachable!()
        };
        let Some((shape, timing)) = tween(first.value, second.value) else {
            continue;
        };
        let (start, duration) = match timing {
            TweenTiming::Early => {
                let duration = (first.duration() / 3).min(MAX_TWEEN_CENTISECONDS);
                (first.end - duration, duration)
            }
            TweenTiming::Centered => {
                let duration = (first.duration() / 4)
                    .min(second.duration() / 4)
                    .min(MAX_TWEEN_CENTISECONDS);
                (first.end - duration / 2, duration)
            }
            TweenTiming::Late => {
                let duration = (second.duration() / 3).min(MAX_TWEEN_CENTISECONDS);
                (second.start, duration)
            }
        };
        if duration >= MIN_TWEEN_CENTISECONDS {
            result.set(start, start + duration, shape);
        }
    }
    result
}

const fn tween(first: MouthShape, second: MouthShape) -> Option<(MouthShape, TweenTiming)> {
    use MouthShape::*;
    use TweenTiming::*;
    match (first, second) {
        (D, A) | (D, G) => Some((C, Early)),
        (D, B) => Some((C, Centered)),
        (D, X) => Some((C, Late)),
        (C, F) | (F, C) | (D, F) => Some((E, Centered)),
        (H, F) => Some((E, Late)),
        (F, H) => Some((E, Early)),
        _ => None,
    }
}

fn avoid_static_segments(rules: &Timeline<ShapeRule>) -> Timeline<MouthShape> {
    let animation = animate_rules_without_static_pass(rules);
    let static_segments = static_segments(rules, &animation);
    if static_segments.is_empty() {
        return animation;
    }

    let mut fixed = rules.clone();
    for (start, end) in static_segments {
        let (extended_start, extended_end) = extend_to_fixed_rules(start, end, rules);
        let segment = fixed_segment_rules(&fixed, extended_start, extended_end);
        for rule in segment.spans() {
            fixed.set(rule.start, rule.end, rule.value);
        }
    }
    animate_rules_without_static_pass(&fixed)
}

fn animate_rules_without_static_pass(rules: &Timeline<ShapeRule>) -> Timeline<MouthShape> {
    animate_rules(rules)
}

fn static_segments(
    rules: &Timeline<ShapeRule>,
    animation: &Timeline<MouthShape>,
) -> Vec<(i64, i64)> {
    animation
        .spans()
        .iter()
        .filter(|shape| {
            shape.duration() >= MIN_STATIC_CENTISECONDS
                && syllable_count(rules, shape.start, shape.end) >= MIN_STATIC_SYLLABLES
        })
        .map(|shape| (shape.start, shape.end))
        .collect()
}

fn syllable_count(rules: &Timeline<ShapeRule>, start: i64, end: i64) -> usize {
    rules
        .spans()
        .iter()
        .filter(|rule| rule.end > start && rule.start < end)
        .filter(|rule| {
            let middle = (rule.value.phone_start + rule.value.phone_end) / 2;
            middle >= start && middle < end && rule.value.phone.is_some_and(Phone::is_vowel)
        })
        .count()
}

fn extend_to_fixed_rules(start: i64, end: i64, rules: &Timeline<ShapeRule>) -> (i64, i64) {
    let spans = rules.spans();
    let mut first = spans
        .iter()
        .position(|span| span.end > start)
        .expect("static segment starts outside shape rules");
    while first > 0 && spans[first].value.shapes.len() > 1 {
        first -= 1;
    }
    let mut last = spans
        .iter()
        .rposition(|span| span.start < end)
        .expect("static segment ends outside shape rules");
    while last + 1 < spans.len() && spans[last].value.shapes.len() > 1 {
        last += 1;
    }
    (spans[first].start, spans[last].end)
}

#[derive(Clone)]
struct Scenario {
    rules: Timeline<ShapeRule>,
    static_count: usize,
    duration_square_sum: i128,
}

impl Scenario {
    fn new(rules: Timeline<ShapeRule>) -> Self {
        let animation = animate_rules_without_static_pass(&rules);
        let static_count = static_segments(&rules, &animation).len();
        let duration_square_sum = animation
            .spans()
            .iter()
            .map(|shape| i128::from(shape.duration()).pow(2))
            .sum();
        Self {
            rules,
            static_count,
            duration_square_sum,
        }
    }

    fn is_better_than(&self, other: &Self) -> bool {
        (self.static_count == 0 && other.static_count != 0)
            || self.duration_square_sum < other.duration_square_sum
    }
}

fn fixed_segment_rules(
    original: &Timeline<ShapeRule>,
    start: i64,
    end: i64,
) -> Timeline<ShapeRule> {
    let mut segment = Timeline::new_unjoined(start, end, ShapeRule::silence());
    for rule in original.clipped_spans(start, end) {
        segment.set(rule.start, rule.end, rule.value);
    }
    let possible: Vec<_> = segment
        .spans()
        .iter()
        .enumerate()
        .filter(|(_, rule)| {
            rule.value.phone.is_some_and(Phone::is_vowel) && rule.value.shapes.len() == 1
        })
        .map(|(index, _)| index)
        .collect();
    let mut best = Scenario::new(segment.clone());
    for count in 1..=possible.len().min(MAX_STATIC_REPLACEMENTS) {
        for combination in combinations(&possible, count) {
            let mut changed = segment.clone();
            for index in combination {
                let rule = segment.spans()[index].clone();
                let mut value = rule.value;
                if value.shapes == ShapeSet::one(MouthShape::B) {
                    value.shapes = ShapeSet::one(MouthShape::C);
                }
                changed.set(rule.start, rule.end, value);
            }
            let candidate = Scenario::new(changed);
            if candidate.is_better_than(&best) {
                best = candidate;
            }
        }
        if best.static_count == 0 {
            break;
        }
    }
    best.rules
}

fn combinations(values: &[usize], count: usize) -> Vec<Vec<usize>> {
    fn collect(
        values: &[usize],
        count: usize,
        offset: usize,
        current: &mut Vec<usize>,
        output: &mut Vec<Vec<usize>>,
    ) {
        if current.len() == count {
            output.push(current.clone());
            return;
        }
        let needed = count - current.len();
        for index in offset..=values.len() - needed {
            current.push(values[index]);
            collect(values, count, index + 1, current, output);
            current.pop();
        }
    }

    let mut output = Vec::new();
    collect(values, count, 0, &mut Vec::new(), &mut output);
    output
}
