use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, generate::SchemaSettings, json_schema};
use serde_json::Value;

use crate::Id;
use crate::types::{
    Color, FORMAT, FileKind, FinishKeys, KeyValue, MaterialsFile, PrefabFile, ProjectFile,
    ProxiesFile, SceneFile, Sound, Sounds, TunableValue,
};

pub fn json_schema(kind: FileKind) -> String {
    let generator = SchemaSettings::draft2020_12().into_generator();
    let mut schema = match kind {
        FileKind::Project => generator.into_root_schema_for::<ProjectFile>(),
        FileKind::Scene => generator.into_root_schema_for::<SceneFile>(),
        FileKind::Prefab => generator.into_root_schema_for::<PrefabFile>(),
        FileKind::Materials => generator.into_root_schema_for::<MaterialsFile>(),
        FileKind::Proxies => generator.into_root_schema_for::<ProxiesFile>(),
        FileKind::Finish => generator.into_root_schema_for::<FinishKeys>(),
    };
    schema.insert(
        "title".into(),
        format!("pfx-scene {}, format {FORMAT}", kind.name()).into(),
    );
    let mut value = schema.to_value();
    no_null(&mut value);
    let mut text = serde_json::to_string_pretty(&value).unwrap_or_default();
    text.push('\n');
    text
}

fn no_null(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(types)) = map.get_mut("type") {
                types.retain(|kind| kind != "null");
                if let [only] = types.as_slice() {
                    let only = only.clone();
                    map.insert("type".into(), only);
                }
            }
            if let Some(Value::Array(any)) = map.get_mut("anyOf") {
                any.retain(|schema| schema.get("type").is_none_or(|kind| kind != "null"));
                if let [only] = any.as_slice() {
                    let only = only.clone();
                    map.remove("anyOf");
                    if let Value::Object(inner) = only {
                        map.extend(inner);
                    }
                }
            }
            if map.get("const").is_some_and(Value::is_null) {
                map.remove("const");
            }
            if let Some(Value::Array(choices)) = map.get_mut("enum") {
                choices.retain(|choice| !choice.is_null());
            }
            map.values_mut().for_each(no_null);
        }
        Value::Array(items) => items.iter_mut().for_each(no_null),
        _ => {}
    }
}

fn numbers(count: usize) -> Schema {
    json_schema!({
        "type": "array",
        "items": { "type": "number" },
        "minItems": count,
        "maxItems": count,
    })
}

impl JsonSchema for Id {
    fn schema_name() -> Cow<'static, str> {
        "Id".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^[0-9a-hjkmnp-tv-z]{10}$",
        })
    }
}

impl JsonSchema for Color {
    fn schema_name() -> Cow<'static, str> {
        "Color".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "anyOf": [
                { "type": "string", "pattern": "^#[0-9a-fA-F]{6}$" },
                numbers(3),
            ]
        })
    }
}

impl JsonSchema for TunableValue {
    fn schema_name() -> Cow<'static, str> {
        "TunableValue".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "anyOf": [
                { "type": "boolean" },
                { "type": "number" },
                numbers(3),
            ]
        })
    }
}

impl JsonSchema for KeyValue {
    fn schema_name() -> Cow<'static, str> {
        "KeyValue".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "anyOf": [
                { "type": "number" },
                numbers(2),
                numbers(3),
            ]
        })
    }
}

impl JsonSchema for Sounds {
    fn schema_name() -> Cow<'static, str> {
        "Sounds".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "anyOf": [
                { "type": "string" },
                {
                    "type": "object",
                    "additionalProperties": generator.subschema_for::<Sound>(),
                },
            ]
        })
    }
}
