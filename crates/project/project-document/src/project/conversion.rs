mod v10_to_v11;
mod v11_to_v12;
mod v12_to_v13;
mod v13_to_v14;
mod v14_to_v15;
mod v15_to_v16;
mod v16_to_v17;
mod v17_to_v18;
mod v18_to_v19;
mod v19_to_v20;
mod v20_to_v21;
mod v21_to_v22;
mod v22_to_v23;
mod v23_to_v24;
mod v24_to_v25;
mod v25_to_v26;
mod v26_to_v27;
mod v27_to_v28;
mod v28_to_v29;
mod v29_to_v30;
mod v30_to_v31;
mod v31_to_v32;

use std::collections::BTreeMap;

use serde_value::Value;

use super::{PROJECT_FORMAT_VERSION, Project};

const FORMAT_VERSION_KEY: &str = "format_version";

trait ProjectVersionConverter: Sync {
    fn source_version(&self) -> u32;
    fn target_version(&self) -> u32;
    fn convert(&self, project: Value) -> Result<Value, String>;
}

static V10_TO_V11: v10_to_v11::Converter = v10_to_v11::Converter;
static V11_TO_V12: v11_to_v12::Converter = v11_to_v12::Converter;
static V12_TO_V13: v12_to_v13::Converter = v12_to_v13::Converter;
static V13_TO_V14: v13_to_v14::Converter = v13_to_v14::Converter;
static V14_TO_V15: v14_to_v15::Converter = v14_to_v15::Converter;
static V15_TO_V16: v15_to_v16::Converter = v15_to_v16::Converter;
static V16_TO_V17: v16_to_v17::Converter = v16_to_v17::Converter;
static V17_TO_V18: v17_to_v18::Converter = v17_to_v18::Converter;
static V18_TO_V19: v18_to_v19::Converter = v18_to_v19::Converter;
static V19_TO_V20: v19_to_v20::Converter = v19_to_v20::Converter;
static V20_TO_V21: v20_to_v21::Converter = v20_to_v21::Converter;
static V21_TO_V22: v21_to_v22::Converter = v21_to_v22::Converter;
static V22_TO_V23: v22_to_v23::Converter = v22_to_v23::Converter;
static V23_TO_V24: v23_to_v24::Converter = v23_to_v24::Converter;
static V24_TO_V25: v24_to_v25::Converter = v24_to_v25::Converter;
static V25_TO_V26: v25_to_v26::Converter = v25_to_v26::Converter;
static V26_TO_V27: v26_to_v27::Converter = v26_to_v27::Converter;
static V27_TO_V28: v27_to_v28::Converter = v27_to_v28::Converter;
static V28_TO_V29: v28_to_v29::Converter = v28_to_v29::Converter;
static V29_TO_V30: v29_to_v30::Converter = v29_to_v30::Converter;
static V30_TO_V31: v30_to_v31::Converter = v30_to_v31::Converter;
static V31_TO_V32: v31_to_v32::Converter = v31_to_v32::Converter;
static CONVERTERS: [&dyn ProjectVersionConverter; 22] = [
    &V10_TO_V11,
    &V11_TO_V12,
    &V12_TO_V13,
    &V13_TO_V14,
    &V14_TO_V15,
    &V15_TO_V16,
    &V16_TO_V17,
    &V17_TO_V18,
    &V18_TO_V19,
    &V19_TO_V20,
    &V20_TO_V21,
    &V21_TO_V22,
    &V22_TO_V23,
    &V23_TO_V24,
    &V24_TO_V25,
    &V25_TO_V26,
    &V26_TO_V27,
    &V27_TO_V28,
    &V28_TO_V29,
    &V29_TO_V30,
    &V30_TO_V31,
    &V31_TO_V32,
];

pub(super) fn from_json(contents: &str) -> Result<Project, String> {
    let value = serde_json::from_str(contents)
        .map_err(|error| format!("could not decode project JSON: {error}"))?;
    let value = to_latest(value)?;
    let bytes = serde_json::to_vec(&value)
        .map_err(|error| format!("could not encode converted project JSON: {error}"))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("could not decode converted project JSON: {error}"))
}

pub(super) fn from_messagepack(bytes: &[u8]) -> Result<Project, String> {
    let value = rmp_serde::from_slice(bytes)
        .map_err(|error| format!("could not decode MessagePack project: {error}"))?;
    let value = to_latest(value)?;
    let bytes = rmp_serde::to_vec_named(&value)
        .map_err(|error| format!("could not encode converted MessagePack project: {error}"))?;
    rmp_serde::from_slice(&bytes)
        .map_err(|error| format!("could not decode converted MessagePack project: {error}"))
}

fn to_latest(mut project: Value) -> Result<Value, String> {
    while project_version(&project)? != PROJECT_FORMAT_VERSION {
        let version = project_version(&project)?;
        let converter = CONVERTERS
            .iter()
            .find(|converter| converter.source_version() == version)
            .ok_or_else(|| {
                format!("unsupported project format {version}; expected {PROJECT_FORMAT_VERSION}")
            })?;
        project = converter.convert(project)?;
        let converted_version = project_version(&project)?;
        if converted_version != converter.target_version() {
            return Err(format!(
                "project converter for format {version} produced format {converted_version} instead of {}",
                converter.target_version()
            ));
        }
    }
    Ok(project)
}

fn project_version(project: &Value) -> Result<u32, String> {
    let Some(value) = project_map_ref(project)?.get(&Value::String(FORMAT_VERSION_KEY.to_string()))
    else {
        return Ok(PROJECT_FORMAT_VERSION);
    };
    match value {
        Value::U8(value) => Ok((*value).into()),
        Value::U16(value) => Ok((*value).into()),
        Value::U32(value) => Ok(*value),
        Value::U64(value) => u32::try_from(*value)
            .map_err(|_| format!("project format version {value} is too large")),
        Value::I8(value) => {
            u32::try_from(*value).map_err(|_| format!("project format version {value} is invalid"))
        }
        Value::I16(value) => {
            u32::try_from(*value).map_err(|_| format!("project format version {value} is invalid"))
        }
        Value::I32(value) => {
            u32::try_from(*value).map_err(|_| format!("project format version {value} is invalid"))
        }
        Value::I64(value) => {
            u32::try_from(*value).map_err(|_| format!("project format version {value} is invalid"))
        }
        _ => Err("project format_version must be an integer".to_string()),
    }
}

fn set_project_version(project: &mut Value, version: u32) -> Result<(), String> {
    project_map(project)?.insert(
        Value::String(FORMAT_VERSION_KEY.to_string()),
        Value::U32(version),
    );
    Ok(())
}

fn ensure_project_version(project: &Value, expected: u32) -> Result<(), String> {
    let version = project_version(project)?;
    if version == expected {
        Ok(())
    } else {
        Err(format!(
            "project converter expected format {expected}, got {version}"
        ))
    }
}

fn project_map(project: &mut Value) -> Result<&mut BTreeMap<Value, Value>, String> {
    match project {
        Value::Map(map) => Ok(map),
        _ => Err("project root must be a map".to_string()),
    }
}

fn project_map_ref(project: &Value) -> Result<&BTreeMap<Value, Value>, String> {
    match project {
        Value::Map(map) => Ok(map),
        _ => Err("project root must be a map".to_string()),
    }
}
