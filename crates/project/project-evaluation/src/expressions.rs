use std::{
    any::TypeId,
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use sanim_lang::{
    Limits,
    core::{Diagnostic, ListLength, Number, Type, Unit, Value, cache::Cache, math},
    expression::{Bindings, CheckedExpression, Interface},
    prelude::Parameter,
};
use shrimply_math_core::{Fraction, motion_noise};
use shrimply_property_model::timeline_value::{TimelineExpressionValue, expression_number};
use uuid::Uuid;

use super::TransformEvaluation;

const MAX_AUDIO_TRACK_ARGUMENTS: usize = 16;
const SLOW_EXPRESSION_LOG_THRESHOLD: Duration = Duration::from_millis(2);
const SOURCE_NAME: &str = "property expression";
const NUMBER: Type = Type::Numeric(Unit::Number);
const DURATION: Type = Type::Numeric(Unit::Duration);
const RATE: Type = Type::Numeric(Unit::Rate);

const INPUTS: &[(&str, Type)] = &[
    ("time", DURATION),
    ("t", DURATION),
    ("local_t", DURATION),
    ("duration", DURATION),
    ("fps", RATE),
    ("canvas_width", NUMBER),
    ("canvas_height", NUMBER),
    ("media_width", NUMBER),
    ("media_height", NUMBER),
    ("source_width", NUMBER),
    ("source_height", NUMBER),
    ("seed", NUMBER),
];
const HOST_FUNCTIONS: &[&str] = &["vol", "mouth", "random", "shake"];

pub fn expression_symbols() -> impl Iterator<Item = &'static str> {
    INPUTS
        .iter()
        .map(|(name, _)| *name)
        .chain(HOST_FUNCTIONS.iter().copied())
        .chain(
            sanim_lang::prelude::all_builtins()
                .filter(|builtin| {
                    !builtin.name.contains('.') && sanim_lang::expression::allows_builtin(builtin)
                })
                .map(|builtin| builtin.name),
        )
        .chain(sanim_core_symbols())
}

fn sanim_core_symbols() -> impl Iterator<Item = &'static str> {
    [
        "value", "let", "mut", "fn", "if", "else", "for", "in", "repeat", "return", "match",
        "case", "and", "or", "not", "true", "false", "None", "Number", "Int", "Bool", "String",
        "Duration", "Rate", "Angle", "List", "Option",
    ]
    .into_iter()
}

fn limits() -> Limits {
    Limits {
        max_operations: 100_000,
        max_native_work: 100_000,
        max_call_depth: 16,
        ..Limits::default()
    }
}

pub struct TransformExpressionCache {
    entries: Cache<Uuid, CachedExpression, Rc<CachedExpression>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpressionDiagnostic {
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

struct CachedExpression {
    source: String,
    property: TypeId,
    checked: Result<CheckedExpression, Diagnostic>,
    last_error: RefCell<Option<String>>,
}

impl Default for TransformExpressionCache {
    fn default() -> Self {
        let limits = Limits::default();
        Self {
            entries: Cache::new(limits.max_cache_bytes, limits.max_cache_entries),
        }
    }
}

impl TransformExpressionCache {
    pub fn syntax_diagnostic(source: &str) -> Option<ExpressionDiagnostic> {
        sanim_lang::expression::parse(SOURCE_NAME, source, &limits())
            .err()
            .map(|error| ExpressionDiagnostic {
                message: error.message,
                line: (error.span.line > 0).then_some(error.span.line),
                column: (error.span.column > 0).then_some(error.span.column),
            })
    }

    pub(crate) fn eval_timeline_value<T: TimelineExpressionValue>(
        &mut self,
        eval: &TransformEvaluation,
        value_id: Uuid,
        source: &str,
        base: &T,
    ) -> Option<T> {
        self.eval_timeline_value_result(eval, value_id, source, base)
            .ok()
    }

    pub fn eval_timeline_value_result<T: TimelineExpressionValue>(
        &mut self,
        eval: &TransformEvaluation,
        value_id: Uuid,
        source: &str,
        base: &T,
    ) -> Result<T, String> {
        let property = TypeId::of::<T>();
        let entry = if let Some(entry) = self.entries.get(&value_id)
            && entry.property == property
            && entry.source == source
        {
            shrimply_profiling::increment("Expression cache / Hit");
            entry
        } else {
            shrimply_profiling::increment("Expression cache / Miss");
            let _measurement = shrimply_profiling::measure("Expression / Check");
            let started = Instant::now();
            let checked = CheckedExpression::check(
                SOURCE_NAME,
                source,
                interface(T::expression_type()),
                limits(),
            );
            if started.elapsed() >= SLOW_EXPRESSION_LOG_THRESHOLD {
                tracing::debug!(%value_id, source_len = source.len(), elapsed_us = started.elapsed().as_micros(), "slow property expression check");
            }
            let bytes = source
                .len()
                .saturating_add(size_of::<CachedExpression>())
                .saturating_add(checked.as_ref().map_or_else(
                    |diagnostic| diagnostic.message.len(),
                    CheckedExpression::retained_bytes,
                ));
            let entry = Rc::new(CachedExpression {
                source: source.into(),
                property,
                checked,
                last_error: RefCell::new(None),
            });
            self.entries.insert(value_id, entry.clone(), bytes);
            entry
        };
        let _measurement = shrimply_profiling::measure("Expression / Evaluate");
        let result = entry
            .checked
            .as_ref()
            .map_err(ToString::to_string)
            .and_then(|program| {
                let frame = Frame::new(eval);
                let input = frame.inputs(program, base)?;
                let value = program
                    .evaluate(input, &|name, args| frame.call(name, args))
                    .map_err(|error| error.to_string())?;
                base.expression_output(value)
                    .ok_or_else(|| "expression returned an invalid property value".to_string())
            });
        if let Err(error) = &result {
            let mut previous = entry.last_error.borrow_mut();
            if previous.as_ref() != Some(error) {
                tracing::warn!(item = %eval.item_id, %value_id, "{error}");
                *previous = Some(error.clone());
            }
        }
        result
    }
}

fn components(ty: &Type) -> &'static [&'static str] {
    match ty {
        Type::Color => &["r", "g", "b", "a"],
        Type::Numeric(Unit::Number) => &["x"],
        Type::List(_, ListLength::Exact(2)) => &["x", "y"],
        Type::List(_, ListLength::Exact(3)) => &["x", "y", "z"],
        _ => &[],
    }
}

fn interface(ty: Type) -> Interface {
    let names = components(&ty);
    let mut interface = Interface::new(ty.clone()).input("value", ty);
    for (name, ty) in INPUTS {
        interface = interface.input(name, ty.clone());
    }
    for name in names {
        interface = interface.input(name, NUMBER);
    }
    let tracks = Parameter {
        name: "tracks",
        ty: Type::List(Box::new(NUMBER), ListLength::Unknown),
        required: false,
    };
    interface
        .function("vol", NUMBER, [tracks.clone()])
        .function("mouth", Type::String, [tracks])
        .function("random", NUMBER, [])
        .function(
            "shake",
            NUMBER,
            [
                Parameter {
                    name: "phase",
                    ty: NUMBER,
                    required: false,
                },
                Parameter {
                    name: "seed",
                    ty: NUMBER,
                    required: false,
                },
            ],
        )
}

struct Frame<'a> {
    evaluation: &'a TransformEvaluation,
    random: Cell<u32>,
    shake_calls: Cell<u32>,
}

impl<'a> Frame<'a> {
    fn new(evaluation: &'a TransformEvaluation) -> Self {
        Self {
            evaluation,
            random: Cell::new(evaluation.seed as u32),
            shake_calls: Cell::new(0),
        }
    }

    fn inputs<T: TimelineExpressionValue>(
        &self,
        program: &CheckedExpression,
        base: &T,
    ) -> Result<Bindings, String> {
        let eval = self.evaluation;
        let mut base_value = None;
        program
            .inputs()
            .map(|name| {
                let value = match name {
                    "time" | "t" | "local_t" => {
                        number(eval.expression_time.seconds, Unit::Duration)
                    }
                    "duration" => number(eval.duration.seconds, Unit::Duration),
                    "fps" => number(eval.fps, Unit::Rate),
                    "canvas_width" => integer(eval.canvas_size.width),
                    "canvas_height" => integer(eval.canvas_size.height),
                    "media_width" | "source_width" => integer(if eval.source_width == 0 {
                        eval.canvas_size.width
                    } else {
                        eval.source_width
                    }),
                    "media_height" | "source_height" => integer(if eval.source_height == 0 {
                        eval.canvas_size.height
                    } else {
                        eval.source_height
                    }),
                    "seed" => integer(eval.seed as u32),
                    name => {
                        if base_value.is_none() {
                            base_value = Some(base.expression_input()?);
                        }
                        let value = base_value.as_ref().expect("prepared expression base");
                        match name {
                            "value" => value.clone(),
                            "x" if matches!(value, Value::Number(_)) => value.clone(),
                            "x" | "y" | "z" => {
                                let index = usize::from(name.as_bytes()[0] - b'x');
                                value
                                    .list()?
                                    .get(index)
                                    .cloned()
                                    .ok_or("missing vector component")?
                            }
                            "r" | "g" | "b" | "a" => {
                                let Value::Color(color) = value else {
                                    return Err("expected a color".into());
                                };
                                let index = ["r", "g", "b", "a"]
                                    .iter()
                                    .position(|channel| *channel == name)
                                    .expect("color channel");
                                number(color.0[index], Unit::Number)
                            }
                            _ => return Err(format!("unknown expression input '{name}'")),
                        }
                    }
                };
                Ok((name.to_owned(), value))
            })
            .collect()
    }

    fn call(&self, name: &str, args: &Bindings) -> Result<Value, String> {
        let eval = self.evaluation;
        match name {
            "random" => {
                let seed = motion_noise::next_seed(self.random.get());
                self.random.set(seed);
                Ok(number(motion_noise::random_sample(seed), Unit::Number))
            }
            "shake" => {
                let phase = args
                    .get("phase")
                    .map(Value::number)
                    .transpose()?
                    .map_or_else(
                        || math::arithmetic("*", eval.expression_time.seconds, Fraction::from(5)),
                        |number| Ok(number.value),
                    )?;
                let stream = if let Some(seed) = args.get("seed") {
                    math::integer_i64(seed.number()?.value).ok_or("shake seed is out of range")?
                        as u32
                } else {
                    let stream = self.shake_calls.get();
                    self.shake_calls.set(stream.wrapping_add(1));
                    stream
                };
                let value =
                    motion_noise::shake_sample(eval.item_seed as u32, math::to_f64(phase)?, stream);
                Ok(number(math::functions::approximate(value)?, Unit::Number))
            }
            "vol" | "mouth" => {
                let indices = track_indices(args.get("tracks"))?;
                if name == "vol" {
                    let value = if indices.is_empty() {
                        eval.volume_mixer.all()
                    } else {
                        eval.volume_mixer
                            .selected(&indices)
                            .map_err(|error| error.to_string())?
                    };
                    match value {
                        shrimply_math_media::VolumeValue::Ready(value) => expression_number(value),
                        shrimply_math_media::VolumeValue::Pending => {
                            Err("audio volume analysis is pending".into())
                        }
                        shrimply_math_media::VolumeValue::Failed(error) => Err(error),
                    }
                } else {
                    let item_id = eval.item_id.as_u128();
                    let value = if indices.is_empty() {
                        eval.mouth_mixer
                            .all(item_id, eval.item_start, eval.item_end)
                    } else {
                        eval.mouth_mixer
                            .selected(&indices, item_id, eval.item_start, eval.item_end)
                            .map_err(|error| error.to_string())?
                    };
                    match value {
                        shrimply_lip_sync::MouthValue::Ready(shape) => {
                            Ok(Value::String(shape.as_str().into()))
                        }
                        shrimply_lip_sync::MouthValue::Pending => {
                            Err("mouth analysis is still loading".into())
                        }
                        shrimply_lip_sync::MouthValue::Failed(error) => Err(error),
                    }
                }
            }
            _ => Err(format!("unknown host function '{name}'")),
        }
    }
}

fn track_indices(value: Option<&Value>) -> Result<Vec<usize>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let tracks = value.list()?;
    if tracks.len() > MAX_AUDIO_TRACK_ARGUMENTS {
        return Err(format!(
            "select at most {MAX_AUDIO_TRACK_ARGUMENTS} audio tracks"
        ));
    }
    tracks.iter().map(|value| value.number()?.usize()).collect()
}

fn number(value: Fraction, unit: Unit) -> Value {
    Value::Number(Number { value, unit })
}

fn integer(value: u32) -> Value {
    Value::Number(Number::scalar(i64::from(value)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FrameMouthMixer, FrameVolumeMixer, MouthShape, MouthValue, resolve_with_error};
    use glam::{Vec2, Vec3};
    use shrimply_math_media::VolumeValue;
    use shrimply_project_document::project::{CanvasSize, Time};
    use shrimply_property_model::timeline_value::{
        TimelineBool, TimelineExpression, TimelineValue,
    };

    fn evaluation() -> TransformEvaluation {
        TransformEvaluation {
            item_id: Uuid::from_u128(1),
            local_time: Time::from_seconds(1),
            expression_time: Time::from_fraction(3, 2),
            duration: Time::from_seconds(3),
            item_start: Time::from_seconds(4),
            item_end: Time::from_seconds(7),
            fps: Fraction::new(30_000_u64, 1_001_u64),
            canvas_size: CanvasSize {
                width: 1920,
                height: 1080,
            },
            source_width: 0,
            source_height: 720,
            volume_mixer: FrameVolumeMixer::silent(3),
            mouth_mixer: FrameMouthMixer::silent(3),
            seed: 0x1234,
            item_seed: 0x2345,
        }
    }

    #[test]
    fn cache_checks_property_types_and_uses_each_frames_inputs() {
        let mut eval = evaluation();
        let mut cache = TransformExpressionCache::default();
        let id = Uuid::from_u128(2);
        let base = Vec2::new(4.0, 8.0);
        let source = "value + [time / 1s, duration / 1s]";
        assert_eq!(
            cache
                .eval_timeline_value_result(&eval, id, source, &base)
                .unwrap(),
            Vec2::new(5.5, 11.0)
        );
        eval = eval.at_local_time(Time::from_seconds(2));
        assert_eq!(
            cache
                .eval_timeline_value_result(&eval, id, source, &base)
                .unwrap(),
            Vec2::new(6.0, 11.0)
        );
        assert!(
            cache
                .eval_timeline_value_result(&eval, id, source, &Vec3::ZERO)
                .is_err()
        );
        assert_eq!(
            cache
                .eval_timeline_value_result(&eval, id, "value + [1, 2, 3]", &Vec3::ZERO)
                .unwrap(),
            Vec3::new(1.0, 2.0, 3.0)
        );
        assert_eq!(
            cache
                .eval_timeline_value_result(&eval, id, "x + source_width / canvas_width", &2.0f32)
                .unwrap(),
            3.0
        );
    }

    #[test]
    fn clip_offsets_and_playback_rates_use_the_existing_expression_clock() {
        use shrimply_project_document::project::{Project, VideoItem};
        let project = Project::default();
        let mut item = VideoItem::text_item(
            project.canvas_size,
            Time::from_seconds(4),
            Time::from_seconds(8),
        );
        item.animation_time_offset = Time::from_fraction(3, 2);
        item.playback_fps = Fraction::new(60_000_u64, 1_001_u64);
        let eval = TransformEvaluation::for_item(&project, &item, Time::from_fraction(9, 2));
        let mut cache = TransformExpressionCache::default();
        let result = cache
            .eval_timeline_value_result(&eval, Uuid::new_v4(), "frame_index(time, fps)", &0_i64)
            .unwrap();
        assert_eq!(result, 119);
    }

    #[test]
    fn invalid_expressions_fall_back_and_revised_source_recovers() {
        let eval = evaluation();
        let mut cache = TransformExpressionCache::default();
        let mut property = TimelineValue::new_const(Vec2::new(4.0, 8.0));
        property.expression = Some(TimelineExpression {
            id: Uuid::new_v4(),
            enabled: true,
            source: "[1, 2, 3]".into(),
        });
        let invalid = resolve_with_error(&property, &eval, &mut cache);
        assert_eq!(invalid.value, Vec2::new(4.0, 8.0));
        assert!(invalid.error.unwrap().contains("List<Number,2>"));
        property.expression.as_mut().unwrap().source = "[x * 2, y + 1]".into();
        let fixed = resolve_with_error(&property, &eval, &mut cache);
        assert_eq!(fixed.value, Vec2::new(8.0, 9.0));
        assert!(fixed.error.is_none());
    }

    #[test]
    fn color_text_booleans_and_choices_round_trip() {
        use shrimply_math_color::{Color, LayerBlendMode};
        let eval = evaluation();
        let mut cache = TransformExpressionCache::default();
        let id = Uuid::new_v4();
        assert_eq!(
            cache
                .eval_timeline_value_result(
                    &eval,
                    id,
                    "Rgb(r, 1, b, a)",
                    &Color::<u8>::from_rgba(255, 0, 0, 127)
                )
                .unwrap(),
            Color::from_rgba(255, 255, 0, 127)
        );
        assert_eq!(
            cache
                .eval_timeline_value_result(
                    &eval,
                    id,
                    r#""{value} {frame_index(time, fps)}""#,
                    &"frame".to_string()
                )
                .unwrap(),
            "frame 44"
        );
        assert!(
            cache
                .eval_timeline_value_result(&eval, id, "not value", &TimelineBool::False)
                .unwrap()
                .get()
        );
        assert_eq!(
            cache
                .eval_timeline_value_result(&eval, id, r#""add""#, &LayerBlendMode::Normal)
                .unwrap(),
            LayerBlendMode::Add
        );
        assert!(
            cache
                .eval_timeline_value_result(&eval, id, r#""unknown""#, &LayerBlendMode::Normal)
                .is_err()
        );
        for value in [i64::MIN, i64::MAX] {
            assert_eq!(
                cache
                    .eval_timeline_value_result(&eval, id, "value", &value)
                    .unwrap(),
                value
            );
        }
        assert_eq!(
            cache
                .eval_timeline_value_result(&eval, id, "-0.1", &7_u32)
                .unwrap(),
            0
        );
    }

    #[test]
    fn audio_track_selection_pending_and_random_are_frame_local() {
        let mut eval = evaluation();
        eval.volume_mixer =
            FrameVolumeMixer::resolving(3, |tracks| if tracks == [0, 2] { 0.75 } else { 0.25 });
        eval.mouth_mixer = FrameMouthMixer::resolving(3, |tracks, item, start, end| {
            assert_eq!(
                (item, start, end),
                (1, Time::from_seconds(4), Time::from_seconds(7))
            );
            if tracks == [1] {
                MouthValue::Ready(MouthShape::A)
            } else {
                MouthValue::Pending
            }
        });
        let mut cache = TransformExpressionCache::default();
        let id = Uuid::new_v4();
        assert_eq!(
            cache
                .eval_timeline_value_result(&eval, id, "let tracks = [0, 2]\nvol(tracks)", &0.0f32)
                .unwrap(),
            0.75
        );
        assert!(
            cache
                .eval_timeline_value_result(&eval, id, "vol([3])", &0.0f32)
                .is_err()
        );
        assert_eq!(
            cache
                .eval_timeline_value_result(&eval, id, "mouth([1])", &String::new())
                .unwrap(),
            "A"
        );
        assert!(
            cache
                .eval_timeline_value_result(&eval, id, "mouth()", &String::new())
                .unwrap_err()
                .contains("loading")
        );
        eval.volume_mixer = eval
            .volume_mixer
            .with_deferred_resolver(|_| VolumeValue::Pending)
            .for_preparation();
        assert!(
            cache
                .eval_timeline_value_result(&eval, id, "vol([1])", &0.0f32)
                .unwrap_err()
                .contains("pending")
        );

        let source = "[random(), random(), shake(seed=seed)]";
        let first = cache
            .eval_timeline_value_result(&eval, id, source, &Vec3::ZERO)
            .unwrap();
        assert_ne!(first.x, first.y);
        assert_eq!(
            cache
                .eval_timeline_value_result(&eval, id, source, &Vec3::ZERO)
                .unwrap(),
            first
        );
    }
}
