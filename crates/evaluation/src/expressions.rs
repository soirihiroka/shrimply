use std::{
    any::TypeId,
    cell::RefCell,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
    time::{Duration, Instant},
};

use hashbrown::{HashMap, HashSet};
use num_traits::{CheckedAdd, CheckedDiv, CheckedMul, CheckedSub, ToPrimitive};
use rhai::{
    AST, Array, Dynamic, Engine, EvalAltResult, FLOAT, INT, Map, OptimizationLevel, ParseError,
    Position, Scope, plugin::*,
};
use shrimply_math_core::{Fraction, Sign};
use uuid::Uuid;

use super::TransformEvaluation;
use shrimply_core::timeline_value::{
    ExpressionData, ExpressionInput as TimelineExpressionInput, TimelineExpressionValue,
};
use shrimply_math_color::Color;
use shrimply_project::project::{Time, fraction_numerator};

const FRACTION_ZERO: Fraction = Fraction::new_raw(0, 1);
const MAX_AUDIO_TRACK_ARGUMENTS: usize = 16;
const RANDOM_HALF_RANGE: INT = 2_147_483_648;
const SLOW_EXPRESSION_LOG_THRESHOLD: Duration = Duration::from_millis(2);
const MAX_EXPRESSION_ARRAY_SIZE: usize = 100_000;
const MAX_EXPRESSION_OPERATIONS: u64 = 100_000;

thread_local! {
    static EXPRESSION_STATE: RefCell<Option<ExpressionState>> = const { RefCell::new(None) };
}

#[derive(Clone)]
struct ExpressionState {
    time: Fraction,
    item_id: u128,
    item_start: Time,
    item_end: Time,
    volume_mixer: shrimply_math_media::FrameVolumeMixer,
    mouth_mixer: shrimply_lip_sync::FrameMouthMixer,
    seed: INT,
    item_seed: INT,
    shake_call: INT,
}

struct ExpressionStateReset(Option<ExpressionState>);

struct EngineInput {
    value: Dynamic,
    y: Option<f32>,
    z: Option<f32>,
    color: Option<Color>,
}

impl ExpressionStateReset {
    fn set(eval: &TransformEvaluation) -> Self {
        let state = ExpressionState {
            time: eval.expression_time.seconds,
            item_id: eval.item_id.as_u128(),
            item_start: eval.item_start,
            item_end: eval.item_end,
            volume_mixer: eval.volume_mixer.clone(),
            mouth_mixer: eval.mouth_mixer.clone(),
            seed: (eval.seed & 0xffff_ffff) as INT,
            item_seed: (eval.item_seed & 0xffff_ffff) as INT,
            shake_call: 0,
        };
        let previous = EXPRESSION_STATE.with(|cell| cell.replace(Some(state)));
        Self(previous)
    }
}

impl Drop for ExpressionStateReset {
    fn drop(&mut self) {
        EXPRESSION_STATE.with(|cell| {
            cell.replace(self.0.take());
        });
    }
}

const EXPRESSION_HELPERS: &str = include_str!("expression_helpers.rhai");

pub struct TransformExpressionCache {
    engine: Rc<Engine>,
    entries: HashMap<ExpressionCacheKey, CachedExpression>,
    sources: HashMap<Uuid, Rc<str>>,
}

#[derive(Clone, Debug)]
pub struct ExpressionDiagnostic {
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct ExpressionCacheKey {
    value_id: Uuid,
    source: Rc<str>,
}

enum CachedExpression {
    Ready(Box<ReadyExpression>),
    Invalid {
        error: String,
        logged_errors: HashSet<String>,
    },
}

struct ReadyExpression {
    engine: Rc<Engine>,
    scope: Scope<'static>,
    ast: AST,
    logged_errors: HashSet<String>,
}

impl Default for TransformExpressionCache {
    fn default() -> Self {
        Self {
            engine: Rc::new(expression_engine()),
            entries: HashMap::new(),
            sources: HashMap::new(),
        }
    }
}

impl TransformExpressionCache {
    pub fn syntax_diagnostic(source: &str) -> Option<ExpressionDiagnostic> {
        match compile_expression_source(&expression_engine(), source) {
            Ok(_) => None,
            Err(error) => Some(expression_diagnostic(&error)),
        }
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
        let expression = self.expression_key(value_id, source);
        let input = engine_input(base.expression_input());
        let entry = self
            .entry(expression)
            .ok_or_else(|| "expression cache entry is unavailable".to_string())?;
        let result = match entry.eval_with(eval, input, expression_data_from_dynamic) {
            Ok(Some(output)) => base
                .expression_output(output)
                .ok_or_else(|| "expression returned an invalid value".to_string()),
            Ok(None) => Err("expression returned unsupported data".to_string()),
            Err(error) => Err(error),
        };
        if let Err(error) = &result {
            entry.log_error(eval.item_id, error);
        }
        result
    }

    fn expression_key(&mut self, value_id: Uuid, source: &str) -> ExpressionCacheKey {
        let source = match self.sources.get(&value_id) {
            Some(cached) if cached.as_ref() == source => Rc::clone(cached),
            _ => {
                let source: Rc<str> = Rc::from(source);
                self.sources.insert(value_id, Rc::clone(&source));
                source
            }
        };
        ExpressionCacheKey { value_id, source }
    }

    fn entry(&mut self, key: ExpressionCacheKey) -> Option<&mut CachedExpression> {
        if !self.entries.contains_key(&key) {
            shrimply_benchmarking::increment("Expression cache / Miss");
            let _measurement = shrimply_benchmarking::measure("Expression / Compile");
            let started = Instant::now();
            let entry = match init_expression_scope(Rc::clone(&self.engine), &key.source) {
                Ok(cached) => CachedExpression::Ready(Box::new(cached)),
                Err(error) => {
                    tracing::warn!("Transform expression error value={}: {error}", key.value_id);
                    let mut logged_errors = HashSet::new();
                    logged_errors.insert(error.clone());
                    CachedExpression::Invalid {
                        error,
                        logged_errors,
                    }
                }
            };
            let elapsed = started.elapsed();
            if elapsed >= SLOW_EXPRESSION_LOG_THRESHOLD {
                tracing::debug!(
                    "transform_expression: compile value={} source_len={} elapsed_us={}",
                    key.value_id,
                    key.source.len(),
                    elapsed.as_micros(),
                );
            }
            self.entries.insert(key.clone(), entry);
        } else {
            shrimply_benchmarking::increment("Expression cache / Hit");
        }
        self.entries.get_mut(&key)
    }
}

fn engine_input(input: TimelineExpressionInput) -> EngineInput {
    EngineInput {
        value: expression_data_dynamic(input.value),
        y: input.y,
        z: input.z,
        color: input.color,
    }
}

fn expression_data_dynamic(value: ExpressionData) -> Dynamic {
    match value {
        ExpressionData::Unit => Dynamic::from(()),
        ExpressionData::Bool(value) => Dynamic::from(value),
        ExpressionData::Number(value) => Dynamic::from(f32_fraction(value)),
        ExpressionData::Integer(value) => Dynamic::from(value as INT),
        ExpressionData::Text(value) => Dynamic::from(value),
        ExpressionData::Array(values) => Dynamic::from(
            values
                .into_iter()
                .map(expression_data_dynamic)
                .collect::<Array>(),
        ),
        ExpressionData::Object(values) => Dynamic::from(
            values
                .into_iter()
                .map(|(key, value)| (key.into(), expression_data_dynamic(value)))
                .collect::<Map>(),
        ),
    }
}

fn expression_data_from_dynamic(value: Dynamic) -> Option<ExpressionData> {
    if value.is_unit() {
        return Some(ExpressionData::Unit);
    }
    if value.is::<bool>() {
        return value.try_cast::<bool>().map(ExpressionData::Bool);
    }
    if value.is::<INT>() {
        return value.try_cast::<INT>().map(ExpressionData::Integer);
    }
    if value.is::<rhai::ImmutableString>() {
        return value
            .try_cast::<rhai::ImmutableString>()
            .map(|value| ExpressionData::Text(value.to_string()));
    }
    if value.is::<Array>() {
        return value.try_cast::<Array>().and_then(|values| {
            values
                .into_iter()
                .map(expression_data_from_dynamic)
                .collect::<Option<Vec<_>>>()
                .map(ExpressionData::Array)
        });
    }
    if value.is::<Map>() {
        return value.try_cast::<Map>().and_then(|values| {
            values
                .into_iter()
                .map(|(key, value)| Some((key.to_string(), expression_data_from_dynamic(value)?)))
                .collect::<Option<std::collections::BTreeMap<_, _>>>()
                .map(ExpressionData::Object)
        });
    }
    rhai_number(value).map(ExpressionData::Number)
}

impl CachedExpression {
    fn eval_with<T>(
        &mut self,
        eval: &TransformEvaluation,
        input: EngineInput,
        convert: impl FnOnce(Dynamic) -> Option<T>,
    ) -> Result<Option<T>, String> {
        let Self::Ready(ready) = self else {
            let Self::Invalid { error, .. } = self else {
                unreachable!();
            };
            return Err(error.clone());
        };
        let _measurement = shrimply_benchmarking::measure("Expression / Evaluate");
        let ready = ready.as_mut();
        set_evaluation_globals(&mut ready.scope, eval, input.value, input.y, input.z);
        if let Some(color) = input.color {
            set_color_globals(&mut ready.scope, color);
        }
        let scope_len = ready.scope.len();
        let _state = ExpressionStateReset::set(eval);
        let result = catch_unwind(AssertUnwindSafe(|| {
            ready
                .engine
                .eval_ast_with_scope::<Dynamic>(&mut ready.scope, &ready.ast)
                .map(convert)
                .map_err(rhai_error)
        }));
        ready.scope.rewind(scope_len);
        result.unwrap_or_else(|_| Err("expression evaluation panicked".to_string()))
    }

    fn log_error(&mut self, item_id: Uuid, error: &str) {
        let logged_errors = match self {
            Self::Ready(ready) => &mut ready.logged_errors,
            Self::Invalid { logged_errors, .. } => logged_errors,
        };
        if logged_errors.insert(error.to_string()) {
            tracing::warn!("Transform expression error item={item_id}: {error}");
        }
    }
}

fn expression_engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .set_fast_operators(false)
        .set_max_call_levels(16)
        .set_max_operations(MAX_EXPRESSION_OPERATIONS)
        .set_max_array_size(MAX_EXPRESSION_ARRAY_SIZE)
        .set_optimization_level(OptimizationLevel::Simple);
    register_fraction_api(&mut engine);
    engine.register_global_module(rhai::exported_module!(color_functions).into());
    engine.register_global_module(rhai::exported_module!(time_functions).into());
    register_volume_api(&mut engine);
    register_mouth_api(&mut engine);
    engine
}

fn register_fraction_api(engine: &mut Engine) {
    engine.register_type_with_name::<Fraction>("Fraction");
    engine.register_global_module(rhai::exported_module!(fraction_functions).into());
}

fn register_volume_api(engine: &mut Engine) {
    for argument_count in 0..=MAX_AUDIO_TRACK_ARGUMENTS {
        engine.register_raw_fn::<FLOAT>(
            "vol",
            vec![TypeId::of::<INT>(); argument_count],
            |_, arguments| {
                let indices = arguments
                    .iter()
                    .map(|argument| {
                        argument
                            .as_int()
                            .expect("vol arguments are registered as INT")
                    })
                    .collect::<Vec<_>>();
                expression_volume(&indices)
            },
        );
    }
}

fn register_mouth_api(engine: &mut Engine) {
    for argument_count in 0..=MAX_AUDIO_TRACK_ARGUMENTS {
        engine.register_raw_fn::<rhai::ImmutableString>(
            "mouth",
            vec![TypeId::of::<INT>(); argument_count],
            |_, arguments| {
                let indices = arguments
                    .iter()
                    .map(|argument| {
                        argument
                            .as_int()
                            .expect("mouth arguments are registered as INT")
                    })
                    .collect::<Vec<_>>();
                expression_mouth(&indices)
            },
        );
    }
}

#[export_module]
mod time_functions {
    use super::*;

    #[rhai_fn(return_raw)]
    pub fn timecode(
        frame: INT,
        frame_rate: Fraction,
        drop_frame: bool,
    ) -> Result<rhai::ImmutableString, Box<EvalAltResult>> {
        let timecode = shrimply_math_core::smpte_timecode(frame, frame_rate, drop_frame)
            .ok_or_else(|| arithmetic_error("could not format timecode"))?;
        Ok(shrimply_math_core::format_smpte_timecode(timecode).into())
    }
}

#[export_module]
mod color_functions {
    use super::*;

    #[rhai_fn(return_raw)]
    pub fn rgb(r: Dynamic, g: Dynamic, b: Dynamic) -> Result<Array, Box<EvalAltResult>> {
        Ok(color_array(shrimply_math_color::Color::<u8>::from_srgb([
            color_number(r)?,
            color_number(g)?,
            color_number(b)?,
        ])))
    }

    #[rhai_fn(return_raw)]
    pub fn rgba(
        r: Dynamic,
        g: Dynamic,
        b: Dynamic,
        a: Dynamic,
    ) -> Result<Array, Box<EvalAltResult>> {
        Ok(color_array(shrimply_math_color::Color::<u8>::from_srgba([
            color_number(r)?,
            color_number(g)?,
            color_number(b)?,
            color_number(a)?,
        ])))
    }

    #[rhai_fn(return_raw)]
    pub fn gray(luminance: Dynamic) -> Result<Array, Box<EvalAltResult>> {
        let luminance = color_number(luminance)?;
        Ok(color_array(shrimply_math_color::Color::<u8>::from_srgb([
            luminance, luminance, luminance,
        ])))
    }

    #[rhai_fn(return_raw)]
    pub fn graya(luminance: Dynamic, alpha: Dynamic) -> Result<Array, Box<EvalAltResult>> {
        let luminance = color_number(luminance)?;
        Ok(color_array(shrimply_math_color::Color::<u8>::from_srgba([
            luminance,
            luminance,
            luminance,
            color_number(alpha)?,
        ])))
    }

    #[rhai_fn(return_raw)]
    pub fn hsv(h: Dynamic, s: Dynamic, v: Dynamic) -> Result<Array, Box<EvalAltResult>> {
        Ok(color_array(shrimply_math_color::Color::<u8>::from_hsv(
            color_number(h)?,
            color_number(s)?,
            color_number(v)?,
        )))
    }

    #[rhai_fn(return_raw)]
    pub fn hsva(
        h: Dynamic,
        s: Dynamic,
        v: Dynamic,
        a: Dynamic,
    ) -> Result<Array, Box<EvalAltResult>> {
        Ok(color_array(shrimply_math_color::Color::<u8>::from_hsva(
            color_number(h)?,
            color_number(s)?,
            color_number(v)?,
            color_number(a)?,
        )))
    }

    #[rhai_fn(return_raw)]
    pub fn oklab(l: Dynamic, a: Dynamic, b: Dynamic) -> Result<Array, Box<EvalAltResult>> {
        Ok(color_array(shrimply_math_color::Color::<u8>::from_oklab(
            color_number(l)?,
            color_number(a)?,
            color_number(b)?,
        )))
    }

    #[rhai_fn(return_raw)]
    pub fn oklaba(
        l: Dynamic,
        a: Dynamic,
        b: Dynamic,
        alpha: Dynamic,
    ) -> Result<Array, Box<EvalAltResult>> {
        Ok(color_array(shrimply_math_color::Color::<u8>::from_oklaba(
            color_number(l)?,
            color_number(a)?,
            color_number(b)?,
            color_number(alpha)?,
        )))
    }
}

#[export_module]
mod fraction_functions {
    use super::*;

    #[rhai_fn(name = "Fraction")]
    pub fn fraction_from_fraction(value: Fraction) -> Fraction {
        super::fraction_from_fraction(value)
    }

    #[rhai_fn(name = "Fraction")]
    pub fn fraction_from_int(value: INT) -> Fraction {
        super::fraction_from_int(value)
    }

    #[rhai_fn(name = "Fraction", return_raw)]
    pub fn fraction_from_float(value: FLOAT) -> Result<Fraction, Box<EvalAltResult>> {
        super::fraction_from_float(value)
    }

    #[rhai_fn(name = "Fraction", return_raw)]
    pub fn fraction_new(numerator: INT, denominator: INT) -> Result<Fraction, Box<EvalAltResult>> {
        super::fraction_new(numerator, denominator)
    }

    #[rhai_fn(name = "+")]
    pub fn fraction_identity(value: Fraction) -> Fraction {
        super::fraction_identity(value)
    }

    #[rhai_fn(name = "+", return_raw)]
    pub fn fraction_add(left: Fraction, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
        super::fraction_add(left, right)
    }

    #[rhai_fn(name = "+", return_raw)]
    pub fn fraction_add_int(left: Fraction, right: INT) -> Result<Fraction, Box<EvalAltResult>> {
        super::fraction_add_int(left, right)
    }

    #[rhai_fn(name = "+", return_raw)]
    pub fn int_add_fraction(left: INT, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
        super::int_add_fraction(left, right)
    }

    #[rhai_fn(name = "+", return_raw)]
    pub fn fraction_add_float(left: Fraction, right: FLOAT) -> Result<FLOAT, Box<EvalAltResult>> {
        super::fraction_float_arithmetic(left, right, |left, right| left + right, "addition")
    }

    #[rhai_fn(name = "+", return_raw)]
    pub fn float_add_fraction(left: FLOAT, right: Fraction) -> Result<FLOAT, Box<EvalAltResult>> {
        super::fraction_float_arithmetic(right, left, |right, left| left + right, "addition")
    }

    #[rhai_fn(name = "-")]
    pub fn fraction_neg(value: Fraction) -> Fraction {
        super::fraction_neg(value)
    }

    #[rhai_fn(name = "-", return_raw)]
    pub fn fraction_sub(left: Fraction, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
        super::fraction_sub(left, right)
    }

    #[rhai_fn(name = "-", return_raw)]
    pub fn fraction_sub_int(left: Fraction, right: INT) -> Result<Fraction, Box<EvalAltResult>> {
        super::fraction_sub_int(left, right)
    }

    #[rhai_fn(name = "-", return_raw)]
    pub fn int_sub_fraction(left: INT, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
        super::int_sub_fraction(left, right)
    }

    #[rhai_fn(name = "-", return_raw)]
    pub fn fraction_sub_float(left: Fraction, right: FLOAT) -> Result<FLOAT, Box<EvalAltResult>> {
        super::finite_float(super::fraction_to_f64(left)? - right, "subtraction")
    }

    #[rhai_fn(name = "-", return_raw)]
    pub fn float_sub_fraction(left: FLOAT, right: Fraction) -> Result<FLOAT, Box<EvalAltResult>> {
        super::finite_float(left - super::fraction_to_f64(right)?, "subtraction")
    }

    #[rhai_fn(name = "*", return_raw)]
    pub fn fraction_mul(left: Fraction, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
        super::fraction_mul(left, right)
    }

    #[rhai_fn(name = "*", return_raw)]
    pub fn fraction_mul_int(left: Fraction, right: INT) -> Result<Fraction, Box<EvalAltResult>> {
        super::fraction_mul_int(left, right)
    }

    #[rhai_fn(name = "*", return_raw)]
    pub fn int_mul_fraction(left: INT, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
        super::int_mul_fraction(left, right)
    }

    #[rhai_fn(name = "*", return_raw)]
    pub fn fraction_mul_float(left: Fraction, right: FLOAT) -> Result<FLOAT, Box<EvalAltResult>> {
        super::fraction_float_arithmetic(left, right, |left, right| left * right, "multiplication")
    }

    #[rhai_fn(name = "*", return_raw)]
    pub fn float_mul_fraction(left: FLOAT, right: Fraction) -> Result<FLOAT, Box<EvalAltResult>> {
        super::fraction_float_arithmetic(right, left, |right, left| left * right, "multiplication")
    }

    #[rhai_fn(name = "/", return_raw)]
    pub fn fraction_div(left: Fraction, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
        super::fraction_div(left, right)
    }

    #[rhai_fn(name = "/", return_raw)]
    pub fn fraction_div_int(left: Fraction, right: INT) -> Result<Fraction, Box<EvalAltResult>> {
        super::fraction_div_int(left, right)
    }

    #[rhai_fn(name = "/", return_raw)]
    pub fn int_div_fraction(left: INT, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
        super::int_div_fraction(left, right)
    }

    #[rhai_fn(name = "/", return_raw)]
    pub fn fraction_div_float(left: Fraction, right: FLOAT) -> Result<FLOAT, Box<EvalAltResult>> {
        super::finite_float(super::fraction_to_f64(left)? / right, "division")
    }

    #[rhai_fn(name = "/", return_raw)]
    pub fn float_div_fraction(left: FLOAT, right: Fraction) -> Result<FLOAT, Box<EvalAltResult>> {
        super::finite_float(left / super::fraction_to_f64(right)?, "division")
    }

    #[rhai_fn(name = "%", return_raw)]
    pub fn fraction_mod(left: Fraction, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
        super::fraction_mod(left, right)
    }

    #[rhai_fn(name = "%", return_raw)]
    pub fn fraction_mod_int(left: Fraction, right: INT) -> Result<Fraction, Box<EvalAltResult>> {
        super::fraction_mod_int(left, right)
    }

    #[rhai_fn(name = "%", return_raw)]
    pub fn int_mod_fraction(left: INT, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
        super::int_mod_fraction(left, right)
    }

    #[rhai_fn(name = "%", return_raw)]
    pub fn fraction_mod_float(left: Fraction, right: FLOAT) -> Result<FLOAT, Box<EvalAltResult>> {
        super::finite_float(super::fraction_to_f64(left)? % right, "remainder")
    }

    #[rhai_fn(name = "%", return_raw)]
    pub fn float_mod_fraction(left: FLOAT, right: Fraction) -> Result<FLOAT, Box<EvalAltResult>> {
        super::finite_float(left % super::fraction_to_f64(right)?, "remainder")
    }

    #[rhai_fn(name = "==")]
    pub fn fraction_eq(left: Fraction, right: Fraction) -> bool {
        super::fraction_eq(left, right)
    }

    #[rhai_fn(name = "==")]
    pub fn fraction_eq_int(left: Fraction, right: INT) -> bool {
        super::fraction_eq_int(left, right)
    }

    #[rhai_fn(name = "==")]
    pub fn int_eq_fraction(left: INT, right: Fraction) -> bool {
        super::int_eq_fraction(left, right)
    }

    #[rhai_fn(name = "==", return_raw)]
    pub fn fraction_eq_float(left: Fraction, right: FLOAT) -> Result<bool, Box<EvalAltResult>> {
        Ok(super::fraction_to_f64(left)? == right)
    }

    #[rhai_fn(name = "==", return_raw)]
    pub fn float_eq_fraction(left: FLOAT, right: Fraction) -> Result<bool, Box<EvalAltResult>> {
        Ok(left == super::fraction_to_f64(right)?)
    }

    #[rhai_fn(name = "!=")]
    pub fn fraction_ne(left: Fraction, right: Fraction) -> bool {
        super::fraction_ne(left, right)
    }

    #[rhai_fn(name = "!=")]
    pub fn fraction_ne_int(left: Fraction, right: INT) -> bool {
        super::fraction_ne_int(left, right)
    }

    #[rhai_fn(name = "!=")]
    pub fn int_ne_fraction(left: INT, right: Fraction) -> bool {
        super::int_ne_fraction(left, right)
    }

    #[rhai_fn(name = "!=", return_raw)]
    pub fn fraction_ne_float(left: Fraction, right: FLOAT) -> Result<bool, Box<EvalAltResult>> {
        Ok(super::fraction_to_f64(left)? != right)
    }

    #[rhai_fn(name = "!=", return_raw)]
    pub fn float_ne_fraction(left: FLOAT, right: Fraction) -> Result<bool, Box<EvalAltResult>> {
        Ok(left != super::fraction_to_f64(right)?)
    }

    #[rhai_fn(name = "<")]
    pub fn fraction_lt(left: Fraction, right: Fraction) -> bool {
        super::fraction_lt(left, right)
    }

    #[rhai_fn(name = "<")]
    pub fn fraction_lt_int(left: Fraction, right: INT) -> bool {
        super::fraction_lt_int(left, right)
    }

    #[rhai_fn(name = "<")]
    pub fn int_lt_fraction(left: INT, right: Fraction) -> bool {
        super::int_lt_fraction(left, right)
    }

    #[rhai_fn(name = "<", return_raw)]
    pub fn fraction_lt_float(left: Fraction, right: FLOAT) -> Result<bool, Box<EvalAltResult>> {
        Ok(super::fraction_to_f64(left)? < right)
    }

    #[rhai_fn(name = "<", return_raw)]
    pub fn float_lt_fraction(left: FLOAT, right: Fraction) -> Result<bool, Box<EvalAltResult>> {
        Ok(left < super::fraction_to_f64(right)?)
    }

    #[rhai_fn(name = "<=")]
    pub fn fraction_le(left: Fraction, right: Fraction) -> bool {
        super::fraction_le(left, right)
    }

    #[rhai_fn(name = "<=")]
    pub fn fraction_le_int(left: Fraction, right: INT) -> bool {
        super::fraction_le_int(left, right)
    }

    #[rhai_fn(name = "<=")]
    pub fn int_le_fraction(left: INT, right: Fraction) -> bool {
        super::int_le_fraction(left, right)
    }

    #[rhai_fn(name = "<=", return_raw)]
    pub fn fraction_le_float(left: Fraction, right: FLOAT) -> Result<bool, Box<EvalAltResult>> {
        Ok(super::fraction_to_f64(left)? <= right)
    }

    #[rhai_fn(name = "<=", return_raw)]
    pub fn float_le_fraction(left: FLOAT, right: Fraction) -> Result<bool, Box<EvalAltResult>> {
        Ok(left <= super::fraction_to_f64(right)?)
    }

    #[rhai_fn(name = ">")]
    pub fn fraction_gt(left: Fraction, right: Fraction) -> bool {
        super::fraction_gt(left, right)
    }

    #[rhai_fn(name = ">")]
    pub fn fraction_gt_int(left: Fraction, right: INT) -> bool {
        super::fraction_gt_int(left, right)
    }

    #[rhai_fn(name = ">")]
    pub fn int_gt_fraction(left: INT, right: Fraction) -> bool {
        super::int_gt_fraction(left, right)
    }

    #[rhai_fn(name = ">", return_raw)]
    pub fn fraction_gt_float(left: Fraction, right: FLOAT) -> Result<bool, Box<EvalAltResult>> {
        Ok(super::fraction_to_f64(left)? > right)
    }

    #[rhai_fn(name = ">", return_raw)]
    pub fn float_gt_fraction(left: FLOAT, right: Fraction) -> Result<bool, Box<EvalAltResult>> {
        Ok(left > super::fraction_to_f64(right)?)
    }

    #[rhai_fn(name = ">=")]
    pub fn fraction_ge(left: Fraction, right: Fraction) -> bool {
        super::fraction_ge(left, right)
    }

    #[rhai_fn(name = ">=")]
    pub fn fraction_ge_int(left: Fraction, right: INT) -> bool {
        super::fraction_ge_int(left, right)
    }

    #[rhai_fn(name = ">=")]
    pub fn int_ge_fraction(left: INT, right: Fraction) -> bool {
        super::int_ge_fraction(left, right)
    }

    #[rhai_fn(name = ">=", return_raw)]
    pub fn fraction_ge_float(left: Fraction, right: FLOAT) -> Result<bool, Box<EvalAltResult>> {
        Ok(super::fraction_to_f64(left)? >= right)
    }

    #[rhai_fn(name = ">=", return_raw)]
    pub fn float_ge_fraction(left: FLOAT, right: Fraction) -> Result<bool, Box<EvalAltResult>> {
        Ok(left >= super::fraction_to_f64(right)?)
    }

    pub fn abs(value: Fraction) -> Fraction {
        super::fraction_abs(value)
    }

    pub fn int(value: Fraction) -> INT {
        super::fraction_to_int(value)
    }

    #[rhai_fn(volatile, return_raw)]
    pub fn sin() -> Result<Fraction, Box<EvalAltResult>> {
        super::current_sin()
    }

    #[rhai_fn(volatile, return_raw)]
    pub fn cos() -> Result<Fraction, Box<EvalAltResult>> {
        super::current_cos()
    }

    #[rhai_fn(volatile, return_raw)]
    pub fn tan() -> Result<Fraction, Box<EvalAltResult>> {
        super::current_tan()
    }

    #[rhai_fn(volatile, return_raw)]
    pub fn random() -> Result<Fraction, Box<EvalAltResult>> {
        super::current_random()
    }

    #[rhai_fn(name = "shake", volatile, return_raw)]
    pub fn shake_default() -> Result<Fraction, Box<EvalAltResult>> {
        super::current_shake_default()
    }

    #[rhai_fn(name = "shake", volatile, return_raw)]
    pub fn shake_with_phase(phase: Dynamic) -> Result<Fraction, Box<EvalAltResult>> {
        super::current_shake(phase, None)
    }

    #[rhai_fn(volatile, return_raw)]
    pub fn shake(phase: Dynamic, seed: INT) -> Result<Fraction, Box<EvalAltResult>> {
        super::current_shake(phase, Some(seed))
    }

    #[rhai_fn(return_raw)]
    pub fn shrimply_sin(value: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
        super::fraction_sin(value)
    }

    #[rhai_fn(return_raw)]
    pub fn shrimply_cos(value: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
        super::fraction_cos(value)
    }

    #[rhai_fn(return_raw)]
    pub fn shrimply_tan(value: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
        super::fraction_tan(value)
    }

    #[rhai_fn(return_raw)]
    pub fn shrimply_sqrt(value: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
        super::fraction_sqrt(value)
    }

    #[rhai_fn(return_raw)]
    pub fn shrimply_pow(value: Fraction, power: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
        super::fraction_pow(value, power)
    }

    pub fn shrimply_random_seed(seed: INT) -> INT {
        super::shrimply_random_seed(seed)
    }

    pub fn shrimply_shake(
        item_seed: INT,
        time: Fraction,
        frequency: Fraction,
        size: Fraction,
        seed: INT,
    ) -> Fraction {
        super::fraction_shake(item_seed, time, frequency, size, seed)
    }
}

fn init_expression_scope(engine: Rc<Engine>, source: &str) -> Result<ReadyExpression, String> {
    let ast = compile_source(&engine, source)?;
    Ok(ReadyExpression {
        engine,
        scope: Scope::new(),
        ast,
        logged_errors: HashSet::new(),
    })
}

fn compile_source(engine: &Engine, source: &str) -> Result<AST, String> {
    compile_expression_source(engine, source).map_err(|error| expression_diagnostic(&error).message)
}

fn compile_expression_source(engine: &Engine, source: &str) -> Result<AST, ParseError> {
    let source = format!("{EXPRESSION_HELPERS}{source}");
    engine.compile(source)
}

fn expression_diagnostic(error: &ParseError) -> ExpressionDiagnostic {
    let position = user_source_position(error.position());
    ExpressionDiagnostic {
        message: error.err_type().to_string(),
        line: position.map(|(line, _)| line),
        column: position.and_then(|(_, column)| column),
    }
}

fn user_source_position(position: Position) -> Option<(usize, Option<usize>)> {
    let line = position.line()?;
    let helper_lines = EXPRESSION_HELPERS.lines().count();
    if line <= helper_lines {
        return None;
    }
    Some((line - helper_lines, position.position()))
}

fn set_evaluation_globals(
    scope: &mut Scope,
    eval: &TransformEvaluation,
    x: Dynamic,
    y: Option<f32>,
    z: Option<f32>,
) {
    let seed = (eval.seed & 0xffff_ffff) as INT;
    let time = eval.expression_time.seconds;
    scope.set_value("time", time);
    scope.set_value("t", time);
    scope.set_value("local_t", time);
    scope.set_value("__shrimply_t", time);

    scope.set_value("value", x.clone());
    scope.set_value("x", x);
    match y {
        Some(y) => scope.set_value("y", f32_fraction(y)),
        None => scope.set_value("y", ()),
    };
    match z {
        Some(z) => scope.set_value("z", f32_fraction(z)),
        None => scope.set_value("z", ()),
    };

    scope.set_value("duration", eval.duration.seconds);
    scope.set_value("fps", eval.fps);
    scope.set_value(
        "canvas_width",
        fraction_from_int(INT::from(eval.canvas_size.width)),
    );
    scope.set_value(
        "canvas_height",
        fraction_from_int(INT::from(eval.canvas_size.height)),
    );

    let media_width = if eval.source_width == 0 {
        eval.canvas_size.width
    } else {
        eval.source_width
    };
    let media_height = if eval.source_height == 0 {
        eval.canvas_size.height
    } else {
        eval.source_height
    };
    let media_width = fraction_from_int(INT::from(media_width));
    let media_height = fraction_from_int(INT::from(media_height));
    scope.set_value("media_width", media_width);
    scope.set_value("media_height", media_height);
    scope.set_value("source_width", media_width);
    scope.set_value("source_height", media_height);
    scope.set_value("seed", seed);
    scope.set_value("__shrimply_seed", seed);
    scope.set_value(
        "__shrimply_item_seed",
        (eval.item_seed & 0xffff_ffff) as INT,
    );
    scope.set_value("__shrimply_shake_call", 0 as INT);
}

fn set_color_globals(scope: &mut Scope, color: Color) {
    scope.set_value(
        "value",
        color
            .to_array()
            .into_iter()
            .map(|channel| Dynamic::from(f32_fraction(channel)))
            .collect::<Array>(),
    );
    for (name, channel) in ["r", "g", "b", "a"].into_iter().zip(color.to_array()) {
        scope.set_value(name, f32_fraction(channel));
    }
}

fn fraction_new(numerator: INT, denominator: INT) -> Result<Fraction, Box<EvalAltResult>> {
    if denominator == 0 {
        return Err(arithmetic_error("Fraction denominator cannot be zero"));
    }
    Fraction::new_generic(Sign::Plus, numerator, denominator)
        .ok_or_else(|| arithmetic_error("could not construct Fraction"))
}

fn fraction_from_fraction(value: Fraction) -> Fraction {
    value
}

fn fraction_from_int(value: INT) -> Fraction {
    Fraction::from(value)
}

fn fraction_from_float(value: FLOAT) -> Result<Fraction, Box<EvalAltResult>> {
    finite_fraction_from_f64(value, "could not construct Fraction")
}

fn fraction_to_f64(value: Fraction) -> Result<f64, Box<EvalAltResult>> {
    value
        .to_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| arithmetic_error("expected a finite number"))
}

fn fraction_float_arithmetic(
    fraction: Fraction,
    float: FLOAT,
    operation: impl FnOnce(FLOAT, FLOAT) -> FLOAT,
    name: &str,
) -> Result<FLOAT, Box<EvalAltResult>> {
    finite_float(operation(fraction_to_f64(fraction)?, float), name)
}

fn finite_float(value: FLOAT, operation: &str) -> Result<FLOAT, Box<EvalAltResult>> {
    value
        .is_finite()
        .then_some(value)
        .ok_or_else(|| arithmetic_error(&format!("{operation} produced a non-finite value")))
}

fn finite_fraction_from_f64(value: f64, error: &str) -> Result<Fraction, Box<EvalAltResult>> {
    let value = value as f32;
    if value.is_finite() {
        Ok(f32_fraction(value))
    } else {
        Err(arithmetic_error(error))
    }
}

fn fraction_identity(value: Fraction) -> Fraction {
    value
}

fn fraction_neg(value: Fraction) -> Fraction {
    -value
}

fn fraction_add(left: Fraction, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
    left.checked_add(&right)
        .ok_or_else(|| arithmetic_error("addition overflow"))
}

fn fraction_add_int(left: Fraction, right: INT) -> Result<Fraction, Box<EvalAltResult>> {
    fraction_add(left, fraction_from_int(right))
}

fn int_add_fraction(left: INT, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
    fraction_add(fraction_from_int(left), right)
}

fn fraction_sub(left: Fraction, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
    left.checked_sub(&right)
        .ok_or_else(|| arithmetic_error("subtraction overflow"))
}

fn fraction_sub_int(left: Fraction, right: INT) -> Result<Fraction, Box<EvalAltResult>> {
    fraction_sub(left, fraction_from_int(right))
}

fn int_sub_fraction(left: INT, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
    fraction_sub(fraction_from_int(left), right)
}

fn fraction_mul(left: Fraction, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
    left.checked_mul(&right)
        .ok_or_else(|| arithmetic_error("multiplication overflow"))
}

fn fraction_mul_int(left: Fraction, right: INT) -> Result<Fraction, Box<EvalAltResult>> {
    fraction_mul(left, fraction_from_int(right))
}

fn int_mul_fraction(left: INT, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
    fraction_mul(fraction_from_int(left), right)
}

fn fraction_div(left: Fraction, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
    checked_fraction_div(left, right)
}

fn fraction_div_int(left: Fraction, right: INT) -> Result<Fraction, Box<EvalAltResult>> {
    checked_fraction_div(left, fraction_from_int(right))
}

fn int_div_fraction(left: INT, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
    checked_fraction_div(fraction_from_int(left), right)
}

fn fraction_mod(left: Fraction, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
    checked_fraction_mod(left, right)
}

fn fraction_mod_int(left: Fraction, right: INT) -> Result<Fraction, Box<EvalAltResult>> {
    checked_fraction_mod(left, fraction_from_int(right))
}

fn int_mod_fraction(left: INT, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
    checked_fraction_mod(fraction_from_int(left), right)
}

fn checked_fraction_div(left: Fraction, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
    if right == FRACTION_ZERO {
        Err(arithmetic_error("division by zero"))
    } else {
        left.checked_div(&right)
            .filter(Fraction::is_finite)
            .ok_or_else(|| arithmetic_error("division overflow"))
    }
}

fn checked_fraction_mod(left: Fraction, right: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
    if right == FRACTION_ZERO {
        Err(arithmetic_error("remainder by zero"))
    } else {
        let quotient = left
            .checked_div(&right)
            .ok_or_else(|| arithmetic_error("remainder overflow"))?
            .trunc();
        let product = quotient
            .checked_mul(&right)
            .ok_or_else(|| arithmetic_error("remainder overflow"))?;
        left.checked_sub(&product)
            .ok_or_else(|| arithmetic_error("remainder overflow"))
    }
}

fn fraction_eq(left: Fraction, right: Fraction) -> bool {
    left == right
}

fn fraction_eq_int(left: Fraction, right: INT) -> bool {
    left == fraction_from_int(right)
}

fn int_eq_fraction(left: INT, right: Fraction) -> bool {
    fraction_from_int(left) == right
}

fn fraction_ne(left: Fraction, right: Fraction) -> bool {
    left != right
}

fn fraction_ne_int(left: Fraction, right: INT) -> bool {
    left != fraction_from_int(right)
}

fn int_ne_fraction(left: INT, right: Fraction) -> bool {
    fraction_from_int(left) != right
}

fn fraction_lt(left: Fraction, right: Fraction) -> bool {
    left < right
}

fn fraction_lt_int(left: Fraction, right: INT) -> bool {
    left < fraction_from_int(right)
}

fn int_lt_fraction(left: INT, right: Fraction) -> bool {
    fraction_from_int(left) < right
}

fn fraction_le(left: Fraction, right: Fraction) -> bool {
    left <= right
}

fn fraction_le_int(left: Fraction, right: INT) -> bool {
    left <= fraction_from_int(right)
}

fn int_le_fraction(left: INT, right: Fraction) -> bool {
    fraction_from_int(left) <= right
}

fn fraction_gt(left: Fraction, right: Fraction) -> bool {
    left > right
}

fn fraction_gt_int(left: Fraction, right: INT) -> bool {
    left > fraction_from_int(right)
}

fn int_gt_fraction(left: INT, right: Fraction) -> bool {
    fraction_from_int(left) > right
}

fn fraction_ge(left: Fraction, right: Fraction) -> bool {
    left >= right
}

fn fraction_ge_int(left: Fraction, right: INT) -> bool {
    left >= fraction_from_int(right)
}

fn int_ge_fraction(left: INT, right: Fraction) -> bool {
    fraction_from_int(left) >= right
}

fn fraction_abs(value: Fraction) -> Fraction {
    value.abs()
}

fn fraction_to_int(value: Fraction) -> INT {
    fraction_numerator(value.trunc()) as INT
}

fn expression_volume(indices: &[INT]) -> Result<FLOAT, Box<EvalAltResult>> {
    with_expression_state(|state| {
        let value = if indices.is_empty() {
            state.volume_mixer.all()
        } else {
            let indices = indices
                .iter()
                .map(|&index| {
                    usize::try_from(index)
                        .map_err(|_| arithmetic_error("audio track index cannot be negative"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            state
                .volume_mixer
                .selected(&indices)
                .map_err(|error| arithmetic_error(&error.to_string()))?
        };
        match value {
            shrimply_math_media::VolumeValue::Ready(value) => Ok(FLOAT::from(value)),
            shrimply_math_media::VolumeValue::Pending => {
                Err(arithmetic_error("audio volume analysis is pending"))
            }
            shrimply_math_media::VolumeValue::Failed(error) => Err(arithmetic_error(&error)),
        }
    })
}
fn expression_mouth(indices: &[INT]) -> Result<rhai::ImmutableString, Box<EvalAltResult>> {
    with_expression_state(|state| {
        let value = if indices.is_empty() {
            state
                .mouth_mixer
                .all(state.item_id, state.item_start, state.item_end)
        } else {
            let indices = indices
                .iter()
                .map(|&index| {
                    usize::try_from(index)
                        .map_err(|_| arithmetic_error("audio track index cannot be negative"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            state
                .mouth_mixer
                .selected(&indices, state.item_id, state.item_start, state.item_end)
                .map_err(|error| arithmetic_error(&error.to_string()))?
        };
        match value {
            shrimply_lip_sync::MouthValue::Ready(shape) => Ok(shape.as_str().into()),
            shrimply_lip_sync::MouthValue::Pending => {
                Err(arithmetic_error("mouth analysis is still loading"))
            }
            shrimply_lip_sync::MouthValue::Failed(error) => Err(arithmetic_error(&error)),
        }
    })
}

fn current_sin() -> Result<Fraction, Box<EvalAltResult>> {
    with_expression_state(|state| fraction_sin(state.time))
}

fn current_cos() -> Result<Fraction, Box<EvalAltResult>> {
    with_expression_state(|state| fraction_cos(state.time))
}

fn current_tan() -> Result<Fraction, Box<EvalAltResult>> {
    with_expression_state(|state| fraction_tan(state.time))
}

fn current_random() -> Result<Fraction, Box<EvalAltResult>> {
    with_expression_state_mut(|state| {
        state.seed = shrimply_random_seed(state.seed);
        fraction_sub_int(fraction_new(state.seed, RANDOM_HALF_RANGE)?, 1)
    })
}

fn current_shake_default() -> Result<Fraction, Box<EvalAltResult>> {
    with_expression_state_mut(|state| {
        let seed = next_shake_seed(state, None)?;
        Ok(f64_fraction_shake_phase(
            state.item_seed,
            fraction_to_f64(state.time)? * 5.0,
            1.0,
            seed,
        ))
    })
}

fn current_shake(phase: Dynamic, seed: Option<INT>) -> Result<Fraction, Box<EvalAltResult>> {
    let phase = dynamic_fraction(phase)?;
    with_expression_state_mut(|state| {
        let seed = next_shake_seed(state, seed)?;
        Ok(fraction_shake_phase(
            state.item_seed,
            phase,
            fraction_from_int(1),
            seed,
        ))
    })
}

fn next_shake_seed(
    state: &mut ExpressionState,
    seed: Option<INT>,
) -> Result<INT, Box<EvalAltResult>> {
    match seed {
        Some(seed) => Ok(seed),
        None => {
            let seed = state.shake_call;
            state.shake_call = state
                .shake_call
                .checked_add(1)
                .ok_or_else(|| arithmetic_error("too many shake calls"))?;
            Ok(seed)
        }
    }
}

fn with_expression_state<T>(
    callback: impl FnOnce(&ExpressionState) -> Result<T, Box<EvalAltResult>>,
) -> Result<T, Box<EvalAltResult>> {
    EXPRESSION_STATE.with(|cell| {
        let state = cell.borrow();
        let state = state
            .as_ref()
            .ok_or_else(|| arithmetic_error("expression context is unavailable"))?;
        callback(state)
    })
}

fn with_expression_state_mut<T>(
    callback: impl FnOnce(&mut ExpressionState) -> Result<T, Box<EvalAltResult>>,
) -> Result<T, Box<EvalAltResult>> {
    EXPRESSION_STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let state = state
            .as_mut()
            .ok_or_else(|| arithmetic_error("expression context is unavailable"))?;
        callback(state)
    })
}

fn dynamic_fraction(value: Dynamic) -> Result<Fraction, Box<EvalAltResult>> {
    if let Some(value) = value.clone().try_cast::<Fraction>() {
        return Ok(value);
    }
    if let Ok(value) = value.as_float() {
        return fraction_from_float(value);
    }
    if let Ok(value) = value.as_int() {
        return Ok(fraction_from_int(value));
    }
    Err(arithmetic_error("expected a number"))
}

fn fraction_sin(value: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
    finite_fraction_from_f64(
        fraction_to_f64(value)?.sin(),
        "sin produced a non-finite value",
    )
}

fn fraction_cos(value: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
    finite_fraction_from_f64(
        fraction_to_f64(value)?.cos(),
        "cos produced a non-finite value",
    )
}

fn fraction_tan(value: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
    finite_fraction_from_f64(
        fraction_to_f64(value)?.tan(),
        "tan produced a non-finite value",
    )
}

fn fraction_sqrt(value: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
    if value < FRACTION_ZERO {
        return Err(arithmetic_error("sqrt requires a non-negative value"));
    }
    finite_fraction_from_f64(
        fraction_to_f64(value)?.sqrt(),
        "sqrt produced a non-finite value",
    )
}

fn fraction_pow(value: Fraction, power: Fraction) -> Result<Fraction, Box<EvalAltResult>> {
    finite_fraction_from_f64(
        fraction_to_f64(value)?.powf(fraction_to_f64(power)?),
        "pow produced a non-finite value",
    )
}

fn fraction_shake(
    item_seed: INT,
    time: Fraction,
    frequency: Fraction,
    size: Fraction,
    seed: INT,
) -> Fraction {
    let time = match fraction_to_f64(time) {
        Ok(time) => time,
        _ => return FRACTION_ZERO,
    };
    let frequency = match fraction_to_f64(frequency) {
        Ok(frequency) => frequency,
        _ => return FRACTION_ZERO,
    };
    let size = match fraction_to_f64(size) {
        Ok(size) => size,
        _ => return FRACTION_ZERO,
    };
    if frequency <= 0.0 {
        return FRACTION_ZERO;
    }
    f64_fraction_shake_phase(item_seed, time * frequency, size, seed)
}

fn fraction_shake_phase(item_seed: INT, phase: Fraction, size: Fraction, seed: INT) -> Fraction {
    let phase = match fraction_to_f64(phase) {
        Ok(phase) => phase,
        _ => return FRACTION_ZERO,
    };
    let size = match fraction_to_f64(size) {
        Ok(size) => size,
        _ => return FRACTION_ZERO,
    };
    f64_fraction_shake_phase(item_seed, phase, size, seed)
}

fn f64_fraction_shake_phase(item_seed: INT, phase: f64, size: f64, seed: INT) -> Fraction {
    if size == 0.0 {
        return FRACTION_ZERO;
    }

    let index = phase.floor() as INT;
    let mut progress = phase - index as f64;
    progress = progress * progress * (3.0 - 2.0 * progress);
    let left = fraction_noise_f64(item_seed, index, seed);
    let right = fraction_noise_f64(item_seed, index + 1, seed);
    finite_fraction_from_f64(
        (left + (right - left) * progress) * size,
        "could not construct Fraction",
    )
    .unwrap_or(FRACTION_ZERO)
}

fn fraction_noise_f64(item_seed: INT, index: INT, offset: INT) -> f64 {
    let mut value = (item_seed as u64)
        .wrapping_add((index as u64).wrapping_mul(374_761_393))
        .wrapping_add((offset as u64).wrapping_mul(668_265_263))
        & 0xffff_ffff;
    value = ((value ^ (value >> 13)).wrapping_mul(1_274_126_177)) & 0xffff_ffff;
    value = (value ^ (value >> 16)) & 0xffff_ffff;
    value as f64 / 2_147_483_648.0 - 1.0
}

fn shrimply_random_seed(seed: INT) -> INT {
    ((seed as u64)
        .wrapping_mul(1_664_525)
        .wrapping_add(1_013_904_223)
        & 0xffff_ffff) as INT
}

fn fraction_from_ratio(numerator: INT, denominator: INT) -> Fraction {
    Fraction::new_generic(Sign::Plus, numerator, denominator).unwrap_or(FRACTION_ZERO)
}

fn f32_fraction(value: f32) -> Fraction {
    let (numerator, denominator) = f32_ratio(value).unwrap_or((0, 1));
    fraction_from_ratio(numerator, denominator)
}

fn color_number(value: Dynamic) -> Result<f32, Box<EvalAltResult>> {
    rhai_number(value).ok_or_else(|| arithmetic_error("color channels must be finite numbers"))
}

fn color_array(color: shrimply_math_color::Color<u8>) -> Array {
    color
        .to_srgba()
        .into_iter()
        .map(|channel| Dynamic::from(f32_fraction(channel)))
        .collect()
}

fn rhai_number(value: Dynamic) -> Option<f32> {
    if let Some(value) = value.clone().try_cast::<Fraction>() {
        return fraction_f32(value);
    }
    if let Ok(value) = value.as_int() {
        return finite_f32(value.to_f32()?);
    }
    if let Ok(value) = value.as_float() {
        return finite_f32(value as f32);
    }
    None
}

fn fraction_f32(value: Fraction) -> Option<f32> {
    finite_f32(value.to_f32()?)
}

fn finite_f32(value: f32) -> Option<f32> {
    value.is_finite().then_some(value)
}

fn rhai_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn arithmetic_error(message: &str) -> Box<EvalAltResult> {
    EvalAltResult::ErrorArithmetic(message.to_string(), Position::NONE).into()
}

fn f32_ratio(value: f32) -> Option<(INT, INT)> {
    if !value.is_finite() {
        return None;
    }
    if value == 0.0 {
        return Some((0, 1));
    }
    let bits = value.to_bits();
    let sign = if bits >> 31 == 0 { 1_i128 } else { -1_i128 };
    let exponent = ((bits >> 23) & 0xff) as i32;
    let fraction = bits & 0x7f_ffff;
    let (mantissa, power) = if exponent == 0 {
        (i128::from(fraction), -149)
    } else {
        (i128::from(fraction | 0x80_0000), exponent - 150)
    };
    let mut numerator = sign * mantissa;
    let mut denominator = 1_i128;
    if power >= 0 {
        numerator = numerator.checked_shl(power as u32)?;
    } else {
        denominator = denominator.checked_shl((-power) as u32)?;
    }
    let divisor = gcd_i128(numerator.abs(), denominator);
    numerator /= divisor;
    denominator /= divisor;
    while numerator > i128::from(INT::MAX)
        || numerator < i128::from(INT::MIN)
        || denominator > i128::from(INT::MAX)
    {
        numerator /= 2;
        denominator = (denominator / 2).max(1);
    }
    Some((numerator as INT, denominator as INT))
}

fn gcd_i128(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        let next = a % b;
        a = b;
        b = next;
    }
    a.max(1)
}
