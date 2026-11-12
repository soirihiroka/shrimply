mod shapes;
pub use shapes::{ShapeKind, ShapeRoundingStrategy};
use std::{collections::BTreeMap, fmt::Debug};

use glam::{Vec2, Vec3};
use num_traits::{Bounded, NumCast, ToPrimitive};
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
    CurveEditPolicy, CurveKeyframeInsert, DiscreteEditPolicy, edit_curve_value,
    edit_discrete_value, insert_curve_keyframe, set_expression_enabled, set_keyframes_enabled,
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

#[derive(Clone, Debug, PartialEq)]
pub enum ExpressionData {
    Unit,
    Bool(bool),
    Number(f32),
    Integer(i64),
    Text(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExpressionInput {
    pub value: ExpressionData,
    pub y: Option<f32>,
    pub z: Option<f32>,
    pub color: Option<Color>,
}

impl ExpressionInput {
    pub fn new(value: ExpressionData) -> Self {
        Self {
            value,
            y: None,
            z: None,
            color: None,
        }
    }

    pub fn y(mut self, value: f32) -> Self {
        self.y = Some(value);
        self
    }

    pub fn z(mut self, value: f32) -> Self {
        self.z = Some(value);
        self
    }

    pub fn color(mut self, value: Color) -> Self {
        self.color = Some(value);
        self
    }
}

/// A timeline value that owns its expression representation and validation.
pub trait TimelineExpressionValue: TimelineValueType {
    fn expression_input(&self) -> ExpressionInput;
    fn expression_output(&self, output: ExpressionData) -> Option<Self>;
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
    fn expression_input(&self) -> ExpressionInput {
        let key = Self::variants()
            .iter()
            .find(|variant| variant.value == *self)
            .expect("rotation order is missing from its variants")
            .key;
        ExpressionInput::new(ExpressionData::Text(key.to_string()))
    }

    fn expression_output(&self, output: ExpressionData) -> Option<Self> {
        let ExpressionData::Text(key) = output else {
            return None;
        };
        Self::variants()
            .iter()
            .find(|variant| variant.key == key.as_str())
            .map(|variant| variant.value)
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
    fn expression_input(&self) -> ExpressionInput {
        let key = Self::variants()
            .iter()
            .find(|variant| variant.value == *self)
            .expect("video sample method is missing from its variants")
            .key;
        ExpressionInput::new(ExpressionData::Text(key.to_string()))
    }

    fn expression_output(&self, output: ExpressionData) -> Option<Self> {
        let ExpressionData::Text(key) = output else {
            return None;
        };
        Self::variants()
            .iter()
            .find(|variant| variant.key == key.as_str())
            .map(|variant| variant.value)
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
    fn expression_input(&self) -> ExpressionInput {
        ExpressionInput::new(ExpressionData::Text(self.clone()))
    }

    fn expression_output(&self, output: ExpressionData) -> Option<Self> {
        match output {
            ExpressionData::Text(value) => Some(value),
            _ => None,
        }
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
    fn expression_input(&self) -> ExpressionInput {
        ExpressionInput::new(ExpressionData::Number(*self))
    }

    fn expression_output(&self, output: ExpressionData) -> Option<Self> {
        expression_number(output)
    }
}

impl TimelineExpressionValue for u32 {
    fn expression_input(&self) -> ExpressionInput {
        ExpressionInput::new(ExpressionData::Number(*self as f32))
    }

    fn expression_output(&self, output: ExpressionData) -> Option<Self> {
        expression_number(output).map(|value| <Self as TimelineScalar>::from_f64(value as f64))
    }
}

impl TimelineExpressionValue for i64 {
    fn expression_input(&self) -> ExpressionInput {
        ExpressionInput::new(ExpressionData::Number(*self as f32))
    }

    fn expression_output(&self, output: ExpressionData) -> Option<Self> {
        expression_number(output).map(|value| <Self as TimelineScalar>::from_f64(value as f64))
    }
}

fn expression_number(output: ExpressionData) -> Option<f32> {
    match output {
        ExpressionData::Number(value) => Some(value),
        ExpressionData::Integer(value) => Some(value as f32),
        _ => None,
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
                NumCast::from(value.round().clamp(minimum, maximum)).unwrap_or_default()
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
    fn expression_input(&self) -> ExpressionInput {
        let color = Color::<f32>::from(*self);
        ExpressionInput::new(ExpressionData::Number(color.r))
            .y(color.g)
            .color(color)
    }

    fn expression_output(&self, output: ExpressionData) -> Option<Self> {
        let ExpressionData::Array(values) = output else {
            return None;
        };
        let numbers = values
            .into_iter()
            .map(|value| match value {
                ExpressionData::Number(value) => Some(value),
                ExpressionData::Integer(value) => Some(value as f32),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        let color = match numbers.as_slice() {
            [luminance] => [*luminance, *luminance, *luminance, 1.0],
            [luminance, alpha] => [*luminance, *luminance, *luminance, *alpha],
            [red, green, blue] => [*red, *green, *blue, 1.0],
            [red, green, blue, alpha] => [*red, *green, *blue, *alpha],
            _ => return None,
        };
        Some(Self::from_srgba(
            color.map(|channel| channel.clamp(0.0, 1.0)),
        ))
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
    fn expression_input(&self) -> ExpressionInput {
        ExpressionInput::new(ExpressionData::Number(self.x)).y(self.y)
    }

    fn expression_output(&self, output: ExpressionData) -> Option<Self> {
        let ExpressionData::Array(values) = output else {
            return None;
        };
        let [x, y] = values.as_slice() else {
            return None;
        };
        Some(Self::new(
            expression_number(x.clone())?,
            expression_number(y.clone())?,
        ))
        .filter(|value| value.is_finite())
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
    fn expression_input(&self) -> ExpressionInput {
        ExpressionInput::new(ExpressionData::Number(self.x))
            .y(self.y)
            .z(self.z)
    }

    fn expression_output(&self, output: ExpressionData) -> Option<Self> {
        let ExpressionData::Array(values) = output else {
            return None;
        };
        let [x, y, z] = values.as_slice() else {
            return None;
        };
        Some(Self::new(
            expression_number(x.clone())?,
            expression_number(y.clone())?,
            expression_number(z.clone())?,
        ))
        .filter(|value| value.is_finite())
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
            fn expression_input(&self) -> $crate::ExpressionInput {
                let key = <Self as $crate::TimelineStep>::variants()
                    .iter()
                    .find(|variant| variant.value == *self)
                    .expect("timeline step value is missing from its variants")
                    .key;
                $crate::ExpressionInput::new($crate::ExpressionData::Text(key.to_string()))
            }

            fn expression_output(&self, output: $crate::ExpressionData) -> Option<Self> {
                let $crate::ExpressionData::Text(key) = output else {
                    return None;
                };
                <Self as $crate::TimelineStep>::variants()
                    .iter()
                    .find(|variant| variant.key == key.as_str())
                    .map(|variant| variant.value)
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
    fn expression_input(&self) -> ExpressionInput {
        ExpressionInput::new(ExpressionData::Bool(self.get()))
    }

    fn expression_output(&self, output: ExpressionData) -> Option<Self> {
        match output {
            ExpressionData::Bool(value) => Some(value.into()),
            _ => None,
        }
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
