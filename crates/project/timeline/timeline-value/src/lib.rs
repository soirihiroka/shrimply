mod shapes;
pub use sanim_core as expression;
use sanim_core::{List, ListLength, Number, Type, Unit, Value};
pub use shapes::{ShapeKind, ShapeRoundingStrategy};
use std::fmt::Debug;

use glam::{Vec2, Vec3};
use num_traits::{Bounded, ToPrimitive};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use shrimply_math_color::{Color, LayerBlendMode};
use shrimply_math_geometry::{ScalarValue, Vector2Value};
use shrimply_render_core::VideoSampleMethod;
use shrimply_transform_3d::{
    RotationOrder, RotationOrderValue, ScalarValue as Scalar3DValue, Vector3Value,
};
use uuid::Uuid;

mod edit;
mod math;

pub use edit::{
    CurveEditPolicy, CurveKeyframeInsert, DiscreteEditPolicy, add_curve_keyframe, edit_curve_value,
    edit_discrete_value, insert_curve_keyframe, remove_keyframes, set_expression_enabled,
    set_expression_source, set_keyframes_enabled,
};
use math::{scalar_value_at, text_value_at};
pub use math::{text_edit_count, vector_value_at};
pub use shrimply_math_interpolation::Interpolation;

pub use shrimply_math_core::{
    FRACTION_ZERO, Fraction, Time, deserialize_fraction, fraction_as_f64, fraction_as_label,
    fraction_denominator, fraction_from_f64, fraction_from_integer, fraction_is_finite,
    fraction_new, fraction_numerator, serialize_fraction,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(bound(
    serialize = "TimelineBase<T>: Serialize",
    deserialize = "TimelineBase<T>: Deserialize<'de>"
))]
pub struct TimelineValue<T: TimelineValueType> {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub base: TimelineBase<T>,
    #[serde(default)]
    pub expression: Option<TimelineExpression>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    rename_all = "snake_case",
    bound(
        serialize = "T: Serialize, T::Keyframe: Serialize",
        deserialize = "T: Deserialize<'de>, T::Keyframe: Deserialize<'de>"
    )
)]
pub enum TimelineBase<T: TimelineValueType> {
    Const(T),
    Keyframes(Vec<T::Keyframe>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimelineExpression {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub enabled: bool,
    pub source: String,
}

impl TimelineExpression {
    pub fn update(&mut self, source: Option<String>, enabled: Option<bool>) -> bool {
        let mut changed = false;
        if let Some(source) = source.filter(|source| *source != self.source) {
            self.source = source;
            changed = true;
        }
        if let Some(enabled) = enabled.filter(|enabled| *enabled != self.enabled) {
            self.enabled = enabled;
            changed = true;
        }
        changed
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>"))]
pub struct TimelineCurveKeyframe<T: TimelineValueType> {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub time: Time,
    pub value: T,
    pub interpolation_to_next: Interpolation,
}

pub type TimelineScalarKeyframe<T> = TimelineCurveKeyframe<T>;
pub type TimelineVectorKeyframe<T> = TimelineCurveKeyframe<T>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>"))]
pub struct TimelineStepKeyframe<T: TimelineValueType> {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub time: Time,
    pub value: T,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimelineTextKeyframe {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub time: Time,
    pub value: String,
    #[serde(default)]
    pub text_interpolation_to_next: TextInterpolation,
    #[serde(default)]
    pub interpolation_to_next: Interpolation,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextInterpolation {
    Jump,
    #[default]
    Type,
    Append,
    Insert,
    Diff,
    Decode,
}

impl TextInterpolation {
    pub const ALL: [Self; 6] = [
        Self::Jump,
        Self::Type,
        Self::Append,
        Self::Insert,
        Self::Diff,
        Self::Decode,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Jump => "Jump",
            Self::Type => "Rewrite",
            Self::Append => "Append",
            Self::Insert => "Insert",
            Self::Diff => "Diff",
            Self::Decode => "Decode",
        }
    }

    pub const fn tooltip(self) -> &'static str {
        match self {
            Self::Jump => "Change all at once",
            Self::Type => "Clear and rewrite the whole text",
            Self::Append => "Edit after the shared beginning",
            Self::Insert => "Edit between the shared ends",
            Self::Diff => "Edit only the changed characters",
            Self::Decode => "Scramble, resize, then reveal the new text",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineBool {
    False,
    #[default]
    True,
}

impl TimelineBool {
    pub const fn get(self) -> bool {
        matches!(self, Self::True)
    }
}

impl std::fmt::Display for TimelineBool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.get(), formatter)
    }
}

impl From<bool> for TimelineBool {
    fn from(value: bool) -> Self {
        if value { Self::True } else { Self::False }
    }
}

pub struct TimelineStepVariant<T> {
    pub value: T,
    pub key: &'static str,
    pub label: &'static str,
    pub icon: Option<&'static str>,
}

pub trait TimelineKeyframe<T>: Clone + Debug + Serialize + DeserializeOwned
where
    T: TimelineValueType,
{
    fn id(&self) -> Uuid;
    fn id_mut(&mut self) -> &mut Uuid;
    fn time(&self) -> Time;
    fn time_mut(&mut self) -> &mut Time;
    fn value(&self) -> &T;
    fn value_mut(&mut self) -> &mut T;
}

pub trait TimelineValueType: Clone + Debug + PartialEq + Sized + 'static {
    type Keyframe: TimelineKeyframe<Self>;

    fn default_value() -> Self;
    fn keyframe(time: Time, value: Self) -> Self::Keyframe;
    fn value_at(keyframes: &[Self::Keyframe], time: Time) -> Self;
}

/// A timeline value owns the static schema and conversion for its Sanim expression.
pub trait TimelineExpressionValue: TimelineValueType {
    fn expression_type() -> Type;
    fn expression_input(&self) -> Result<Value, String>;
    fn expression_output(&self, output: Value) -> Option<Self>;
}

pub fn expression_number(value: f32) -> Result<Value, String> {
    let value = f64::from(value);
    let fraction = sanim_core::math::from_f64(value)
        .or_else(|_| sanim_core::math::functions::approximate(value))?;
    Ok(Value::Number(Number {
        value: fraction,
        unit: Unit::Number,
    }))
}

pub fn expression_f32(value: &Value) -> Option<f32> {
    let number = value.number().ok()?;
    if number.unit != Unit::Number {
        return None;
    }
    let value = sanim_core::math::to_f64(number.value).ok()? as f32;
    value.is_finite().then_some(value)
}

fn vector_expression_type(length: usize) -> Type {
    Type::List(
        Box::new(Type::Numeric(Unit::Number)),
        ListLength::Exact(length),
    )
}

fn vector_expression(values: &[f32]) -> Result<Value, String> {
    let values = values
        .iter()
        .copied()
        .map(expression_number)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::List(List::typed(
        Type::Numeric(Unit::Number),
        values,
    )))
}

pub trait TimelineScalar: TimelineValueType<Keyframe = TimelineCurveKeyframe<Self>> {
    fn to_f64(&self) -> f64;
    fn from_f64(value: f64) -> Self;
}

pub trait TimelineVector: TimelineValueType<Keyframe = TimelineCurveKeyframe<Self>> {
    fn distance(left: &Self, right: &Self) -> f64;
    fn mix(left: &Self, right: &Self, progress: f64) -> Self;
}

pub trait TimelineStep:
    TimelineValueType<Keyframe = TimelineStepKeyframe<Self>> + Copy + Eq
{
    fn variants() -> &'static [TimelineStepVariant<Self>];

    fn expression_variant(self) -> Value {
        let key = Self::variants()
            .iter()
            .find(|variant| variant.value == self)
            .expect("timeline step value is missing from its variants")
            .key;
        Value::String(key.into())
    }

    fn from_expression_variant(output: &Value) -> Option<Self> {
        let key = output.string().ok()?;
        Self::variants()
            .iter()
            .find(|variant| variant.key == key)
            .map(|variant| variant.value)
    }
}

impl<T: TimelineValueType> TimelineValue<T> {
    pub fn new(base: TimelineBase<T>) -> Self {
        Self {
            id: Uuid::new_v4(),
            base,
            expression: None,
        }
    }

    pub fn new_const(value: T) -> Self {
        Self::new(TimelineBase::Const(value))
    }

    pub fn fallback(&self) -> T {
        match &self.base {
            TimelineBase::Const(value) => value.clone(),
            TimelineBase::Keyframes(keyframes) => keyframes
                .first()
                .map(|keyframe| keyframe.value().clone())
                .unwrap_or_else(T::default_value),
        }
    }

    pub fn value_at(&self, time: Time) -> T {
        match &self.base {
            TimelineBase::Const(value) => value.clone(),
            TimelineBase::Keyframes(keyframes) => T::value_at(keyframes, time),
        }
    }

    pub fn expression_source(&self) -> Option<&str> {
        self.expression
            .as_ref()
            .map(|expression| expression.source.as_str())
    }
}

impl<T: TimelineValueType> Default for TimelineValue<T> {
    fn default() -> Self {
        Self::new_const(T::default_value())
    }
}

pub fn deserialize_timeline_value<'de, D, T>(deserializer: D) -> Result<TimelineValue<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: TimelineValueType + Deserialize<'de>,
    T::Keyframe: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(
        untagged,
        bound(
            deserialize = "T: TimelineValueType + Deserialize<'de>, T::Keyframe: Deserialize<'de>"
        )
    )]
    enum Stored<T: TimelineValueType> {
        Timeline(TimelineValue<T>),
        Plain(T),
    }

    Ok(match Stored::deserialize(deserializer)? {
        Stored::Timeline(value) => value,
        Stored::Plain(value) => TimelineValue::new_const(value),
    })
}

impl Vector2Value for TimelineValue<glam::Vec2> {
    fn constant(value: Vec2) -> Self {
        Self::new_const(value)
    }

    fn fallback(&self) -> Vec2 {
        self.fallback()
    }
}

impl Vector3Value for TimelineValue<glam::Vec3> {
    fn constant(value: Vec3) -> Self {
        Self::new_const(value)
    }

    fn fallback(&self) -> Vec3 {
        self.fallback()
    }
}

impl RotationOrderValue for TimelineValue<RotationOrder> {
    fn constant(value: RotationOrder) -> Self {
        Self::new_const(value)
    }

    fn fallback(&self) -> RotationOrder {
        self.fallback()
    }
}

impl Scalar3DValue for TimelineValue<f32> {
    fn constant(value: f32) -> Self {
        Self::new_const(value)
    }

    fn fallback(&self) -> f32 {
        self.fallback()
    }
}

impl TimelineValueType for RotationOrder {
    type Keyframe = TimelineStepKeyframe<Self>;

    fn default_value() -> Self {
        Self::default()
    }

    fn keyframe(time: Time, value: Self) -> Self::Keyframe {
        TimelineStepKeyframe {
            id: Uuid::new_v4(),
            time,
            value,
        }
    }

    fn value_at(keyframes: &[Self::Keyframe], time: Time) -> Self {
        step_value_at(keyframes, time)
    }
}

impl TimelineStep for RotationOrder {
    fn variants() -> &'static [TimelineStepVariant<Self>] {
        &[
            TimelineStepVariant {
                value: Self::Xyz,
                key: "xyz",
                label: "XYZ",
                icon: None,
            },
            TimelineStepVariant {
                value: Self::Xzy,
                key: "xzy",
                label: "XZY",
                icon: None,
            },
            TimelineStepVariant {
                value: Self::Yxz,
                key: "yxz",
                label: "YXZ",
                icon: None,
            },
            TimelineStepVariant {
                value: Self::Yzx,
                key: "yzx",
                label: "YZX",
                icon: None,
            },
            TimelineStepVariant {
                value: Self::Zxy,
                key: "zxy",
                label: "ZXY",
                icon: None,
            },
            TimelineStepVariant {
                value: Self::Zyx,
                key: "zyx",
                label: "ZYX",
                icon: None,
            },
        ]
    }
}

impl TimelineExpressionValue for RotationOrder {
    fn expression_type() -> Type {
        Type::String
    }

    fn expression_input(&self) -> Result<Value, String> {
        Ok(self.expression_variant())
    }

    fn expression_output(&self, output: Value) -> Option<Self> {
        Self::from_expression_variant(&output)
    }
}

impl TimelineValueType for VideoSampleMethod {
    type Keyframe = TimelineStepKeyframe<Self>;

    fn default_value() -> Self {
        Self::default()
    }

    fn keyframe(time: Time, value: Self) -> Self::Keyframe {
        TimelineStepKeyframe {
            id: Uuid::new_v4(),
            time,
            value,
        }
    }

    fn value_at(keyframes: &[Self::Keyframe], time: Time) -> Self {
        step_value_at(keyframes, time)
    }
}

impl TimelineStep for VideoSampleMethod {
    fn variants() -> &'static [TimelineStepVariant<Self>] {
        &[
            TimelineStepVariant {
                value: Self::Nearest,
                key: "nearest",
                label: "Nearest",
                icon: None,
            },
            TimelineStepVariant {
                value: Self::Bilinear,
                key: "bilinear",
                label: "Bilinear",
                icon: None,
            },
            TimelineStepVariant {
                value: Self::Bicubic,
                key: "bicubic",
                label: "Bicubic",
                icon: None,
            },
            TimelineStepVariant {
                value: Self::Mitchell,
                key: "mitchell",
                label: "Mitchell",
                icon: None,
            },
            TimelineStepVariant {
                value: Self::Lanczos2,
                key: "lanczos2",
                label: "Lanczos 2",
                icon: None,
            },
            TimelineStepVariant {
                value: Self::Fsr1Easu,
                key: "fsr1_easu",
                label: "AMD FSR 1 EASU",
                icon: None,
            },
            TimelineStepVariant {
                value: Self::NvidiaImageScaling,
                key: "nvidia_image_scaling",
                label: "NVIDIA Image Scaling",
                icon: None,
            },
            TimelineStepVariant {
                value: Self::Anime4k,
                key: "anime4k",
                label: "Anime4K",
                icon: None,
            },
            TimelineStepVariant {
                value: Self::Anime4kSrgan,
                key: "anime4k_srgan",
                label: "Anime4K SRGAN UUL",
                icon: None,
            },
            TimelineStepVariant {
                value: Self::Xbrz,
                key: "xbrz",
                label: "xBRZ",
                icon: None,
            },
            TimelineStepVariant {
                value: Self::Lanczos3,
                key: "lanczos3",
                label: "Lanczos 3",
                icon: None,
            },
        ]
    }
}

impl TimelineExpressionValue for VideoSampleMethod {
    fn expression_type() -> Type {
        Type::String
    }

    fn expression_input(&self) -> Result<Value, String> {
        Ok(self.expression_variant())
    }

    fn expression_output(&self, output: Value) -> Option<Self> {
        Self::from_expression_variant(&output)
    }
}

impl ScalarValue for TimelineValue<f32> {
    fn constant(value: f32) -> Self {
        Self::new_const(value)
    }

    fn fallback(&self) -> f32 {
        self.fallback()
    }
}

macro_rules! impl_keyframe {
    ($keyframe:ident) => {
        impl<T> TimelineKeyframe<T> for $keyframe<T>
        where
            T: TimelineValueType<Keyframe = Self> + Serialize + DeserializeOwned,
        {
            fn id(&self) -> Uuid {
                self.id
            }

            fn id_mut(&mut self) -> &mut Uuid {
                &mut self.id
            }

            fn time(&self) -> Time {
                self.time
            }

            fn time_mut(&mut self) -> &mut Time {
                &mut self.time
            }

            fn value(&self) -> &T {
                &self.value
            }

            fn value_mut(&mut self) -> &mut T {
                &mut self.value
            }
        }
    };
}

impl_keyframe!(TimelineCurveKeyframe);
impl_keyframe!(TimelineStepKeyframe);

impl TimelineKeyframe<String> for TimelineTextKeyframe {
    fn id(&self) -> Uuid {
        self.id
    }

    fn id_mut(&mut self) -> &mut Uuid {
        &mut self.id
    }

    fn time(&self) -> Time {
        self.time
    }

    fn time_mut(&mut self) -> &mut Time {
        &mut self.time
    }

    fn value(&self) -> &String {
        &self.value
    }

    fn value_mut(&mut self) -> &mut String {
        &mut self.value
    }
}

impl TimelineValueType for String {
    type Keyframe = TimelineTextKeyframe;

    fn default_value() -> Self {
        Self::new()
    }

    fn keyframe(time: Time, value: Self) -> Self::Keyframe {
        TimelineTextKeyframe {
            id: Uuid::new_v4(),
            time,
            value,
            text_interpolation_to_next: TextInterpolation::default(),
            interpolation_to_next: Interpolation::default(),
        }
    }

    fn value_at(keyframes: &[Self::Keyframe], time: Time) -> Self {
        text_value_at(keyframes, time)
    }
}

impl TimelineExpressionValue for String {
    fn expression_type() -> Type {
        Type::String
    }

    fn expression_input(&self) -> Result<Value, String> {
        Ok(Value::String(self.clone().into()))
    }

    fn expression_output(&self, output: Value) -> Option<Self> {
        output.string().ok().map(str::to_owned)
    }
}

macro_rules! scalar_type {
    ($ty:ty, $default:expr) => {
        impl TimelineValueType for $ty {
            type Keyframe = TimelineCurveKeyframe<Self>;

            fn default_value() -> Self {
                $default
            }

            fn keyframe(time: Time, value: Self) -> Self::Keyframe {
                TimelineCurveKeyframe {
                    id: Uuid::new_v4(),
                    time,
                    value,
                    interpolation_to_next: Interpolation::default(),
                }
            }

            fn value_at(keyframes: &[Self::Keyframe], time: Time) -> Self {
                scalar_value_at(keyframes, time)
            }
        }
    };
}

scalar_type!(f32, 0.0);
scalar_type!(u32, 0);
scalar_type!(i64, 0);

impl TimelineScalar for f32 {
    fn to_f64(&self) -> f64 {
        *self as f64
    }

    fn from_f64(value: f64) -> Self {
        value as f32
    }
}

impl TimelineExpressionValue for f32 {
    fn expression_type() -> Type {
        Type::Numeric(Unit::Number)
    }

    fn expression_input(&self) -> Result<Value, String> {
        expression_number(*self)
    }

    fn expression_output(&self, output: Value) -> Option<Self> {
        expression_f32(&output)
    }
}

impl TimelineExpressionValue for u32 {
    fn expression_type() -> Type {
        Type::Numeric(Unit::Number)
    }

    fn expression_input(&self) -> Result<Value, String> {
        Ok(Value::Number(Number::scalar(i64::from(*self))))
    }

    fn expression_output(&self, output: Value) -> Option<Self> {
        let value = output
            .number()
            .ok()?
            .value
            .round()
            .clamp(Fraction::from(0), Fraction::from(u32::MAX));
        u32::try_from(sanim_core::math::integer_i64(value)?).ok()
    }
}

impl TimelineExpressionValue for i64 {
    fn expression_type() -> Type {
        Type::Numeric(Unit::Number)
    }

    fn expression_input(&self) -> Result<Value, String> {
        Ok(Value::Number(Number {
            value: fraction_from_integer(*self),
            unit: Unit::Number,
        }))
    }

    fn expression_output(&self, output: Value) -> Option<Self> {
        let value = output.number().ok()?.value.round().clamp(
            fraction_from_integer(i64::MIN),
            fraction_from_integer(i64::MAX),
        );
        sanim_core::math::integer_i64(value)
    }
}

macro_rules! integer_scalar {
    ($ty:ty) => {
        impl TimelineScalar for $ty {
            fn to_f64(&self) -> f64 {
                ToPrimitive::to_f64(self).unwrap_or_default()
            }

            fn from_f64(value: f64) -> Self {
                let minimum =
                    ToPrimitive::to_f64(&<$ty as Bounded>::min_value()).unwrap_or(f64::MIN);
                let maximum =
                    ToPrimitive::to_f64(&<$ty as Bounded>::max_value()).unwrap_or(f64::MAX);
                num_traits::NumCast::from(value.round().clamp(minimum, maximum)).unwrap_or_default()
            }
        }
    };
}

integer_scalar!(u32);
integer_scalar!(i64);

impl TimelineValueType for Color<u8> {
    type Keyframe = TimelineCurveKeyframe<Self>;

    fn default_value() -> Self {
        Self::BLACK
    }

    fn keyframe(time: Time, value: Self) -> Self::Keyframe {
        TimelineCurveKeyframe {
            id: Uuid::new_v4(),
            time,
            value,
            interpolation_to_next: Interpolation::default(),
        }
    }

    fn value_at(keyframes: &[Self::Keyframe], time: Time) -> Self {
        vector_value_at(keyframes, time)
    }
}

impl TimelineVector for Color<u8> {
    fn distance(left: &Self, right: &Self) -> f64 {
        left.oklaba_distance(*right) as f64
    }

    fn mix(left: &Self, right: &Self, progress: f64) -> Self {
        left.mix_oklaba(*right, progress)
    }
}

impl TimelineExpressionValue for Color<u8> {
    fn expression_type() -> Type {
        Type::Color
    }

    fn expression_input(&self) -> Result<Value, String> {
        Ok(Value::Color(sanim_core::Color(self.to_array().map(
            |channel| Fraction::new(u64::from(channel), u64::from(u8::MAX)),
        ))))
    }

    fn expression_output(&self, output: Value) -> Option<Self> {
        let Value::Color(color) = output else {
            return None;
        };
        color.srgba().ok().map(Self::from_srgba)
    }
}

impl TimelineValueType for Vec2 {
    type Keyframe = TimelineCurveKeyframe<Self>;

    fn default_value() -> Self {
        Self::ZERO
    }

    fn keyframe(time: Time, value: Self) -> Self::Keyframe {
        TimelineCurveKeyframe {
            id: Uuid::new_v4(),
            time,
            value,
            interpolation_to_next: Interpolation::default(),
        }
    }

    fn value_at(keyframes: &[Self::Keyframe], time: Time) -> Self {
        vector_value_at(keyframes, time)
    }
}

impl TimelineVector for Vec2 {
    fn distance(left: &Self, right: &Self) -> f64 {
        left.distance(*right) as f64
    }

    fn mix(left: &Self, right: &Self, progress: f64) -> Self {
        left.lerp(*right, progress as f32)
    }
}

impl TimelineExpressionValue for Vec2 {
    fn expression_type() -> Type {
        vector_expression_type(2)
    }

    fn expression_input(&self) -> Result<Value, String> {
        vector_expression(&self.to_array())
    }

    fn expression_output(&self, output: Value) -> Option<Self> {
        let list = output.list().ok()?;
        (list.len() == 2).then_some(())?;
        Some(Self::new(
            expression_f32(list.get(0)?)?,
            expression_f32(list.get(1)?)?,
        ))
    }
}

impl TimelineValueType for Vec3 {
    type Keyframe = TimelineCurveKeyframe<Self>;

    fn default_value() -> Self {
        Self::ZERO
    }

    fn keyframe(time: Time, value: Self) -> Self::Keyframe {
        TimelineCurveKeyframe {
            id: Uuid::new_v4(),
            time,
            value,
            interpolation_to_next: Interpolation::default(),
        }
    }

    fn value_at(keyframes: &[Self::Keyframe], time: Time) -> Self {
        vector_value_at(keyframes, time)
    }
}

impl TimelineVector for Vec3 {
    fn distance(left: &Self, right: &Self) -> f64 {
        left.distance(*right) as f64
    }

    fn mix(left: &Self, right: &Self, progress: f64) -> Self {
        left.lerp(*right, progress as f32)
    }
}

impl TimelineExpressionValue for Vec3 {
    fn expression_type() -> Type {
        vector_expression_type(3)
    }

    fn expression_input(&self) -> Result<Value, String> {
        vector_expression(&self.to_array())
    }

    fn expression_output(&self, output: Value) -> Option<Self> {
        let list = output.list().ok()?;
        (list.len() == 3).then_some(())?;
        Some(Self::new(
            expression_f32(list.get(0)?)?,
            expression_f32(list.get(1)?)?,
            expression_f32(list.get(2)?)?,
        ))
    }
}

#[macro_export]
macro_rules! timeline_step_type {
    ($ty:ty, $default:expr, $variants:expr) => {
        impl $crate::TimelineValueType for $ty {
            type Keyframe = $crate::TimelineStepKeyframe<Self>;

            fn default_value() -> Self {
                $default
            }

            fn keyframe(time: $crate::Time, value: Self) -> Self::Keyframe {
                $crate::TimelineStepKeyframe {
                    id: uuid::Uuid::new_v4(),
                    time,
                    value,
                }
            }

            fn value_at(keyframes: &[Self::Keyframe], time: $crate::Time) -> Self {
                $crate::step_value_at(keyframes, time)
            }
        }

        impl $crate::TimelineStep for $ty {
            fn variants() -> &'static [$crate::TimelineStepVariant<Self>] {
                $variants
            }
        }

        impl $crate::TimelineExpressionValue for $ty {
            fn expression_type() -> $crate::expression::Type {
                $crate::expression::Type::String
            }

            fn expression_input(&self) -> Result<$crate::expression::Value, String> {
                Ok(<Self as $crate::TimelineStep>::expression_variant(*self))
            }

            fn expression_output(&self, output: $crate::expression::Value) -> Option<Self> {
                <Self as $crate::TimelineStep>::from_expression_variant(&output)
            }
        }
    };
}

timeline_step_type!(
    LayerBlendMode,
    LayerBlendMode::Normal,
    &[
        TimelineStepVariant {
            value: LayerBlendMode::Normal,
            key: "normal",
            label: "Normal",
            icon: None,
        },
        TimelineStepVariant {
            value: LayerBlendMode::Add,
            key: "add",
            label: "Add",
            icon: None,
        },
        TimelineStepVariant {
            value: LayerBlendMode::Multiply,
            key: "multiply",
            label: "Multiply",
            icon: None,
        },
        TimelineStepVariant {
            value: LayerBlendMode::Screen,
            key: "screen",
            label: "Screen",
            icon: None,
        },
        TimelineStepVariant {
            value: LayerBlendMode::Overlay,
            key: "overlay",
            label: "Overlay",
            icon: None,
        },
        TimelineStepVariant {
            value: LayerBlendMode::Darken,
            key: "darken",
            label: "Darken",
            icon: None,
        },
        TimelineStepVariant {
            value: LayerBlendMode::Lighten,
            key: "lighten",
            label: "Lighten",
            icon: None,
        },
        TimelineStepVariant {
            value: LayerBlendMode::Difference,
            key: "difference",
            label: "Difference",
            icon: None,
        },
    ]
);

impl TimelineValueType for TimelineBool {
    type Keyframe = TimelineStepKeyframe<Self>;

    fn default_value() -> Self {
        Self::True
    }

    fn keyframe(time: Time, value: Self) -> Self::Keyframe {
        TimelineStepKeyframe {
            id: Uuid::new_v4(),
            time,
            value,
        }
    }

    fn value_at(keyframes: &[Self::Keyframe], time: Time) -> Self {
        step_value_at(keyframes, time)
    }
}

impl TimelineStep for TimelineBool {
    fn variants() -> &'static [TimelineStepVariant<Self>] {
        &[
            TimelineStepVariant {
                value: TimelineBool::False,
                key: "false",
                label: "Off",
                icon: Some("view-conceal-symbolic"),
            },
            TimelineStepVariant {
                value: TimelineBool::True,
                key: "true",
                label: "On",
                icon: Some("view-reveal-symbolic"),
            },
        ]
    }
}

impl TimelineExpressionValue for TimelineBool {
    fn expression_type() -> Type {
        Type::Bool
    }

    fn expression_input(&self) -> Result<Value, String> {
        Ok(Value::Bool(self.get()))
    }

    fn expression_output(&self, output: Value) -> Option<Self> {
        output.boolean().ok().map(Self::from)
    }
}

pub fn step_value_at<T: TimelineStep>(keyframes: &[TimelineStepKeyframe<T>], time: Time) -> T {
    keyframes
        .iter()
        .rev()
        .find(|keyframe| keyframe.time <= time)
        .or_else(|| keyframes.first())
        .map(|keyframe| keyframe.value)
        .unwrap_or_else(T::default_value)
}
