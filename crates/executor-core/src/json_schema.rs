//! Minimal JSON Schema validation (type, required, properties, enum, items).

use serde_json::Value;
use thiserror::Error;

/// Schema mismatch.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{0}")]
pub struct SchemaError(pub String);

impl SchemaError {
    fn at(path: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        if path.is_empty() {
            Self(message)
        } else {
            Self(format!("{path}: {message}"))
        }
    }
}

/// Validate `instance` against a JSON Schema subset.
///
/// # Errors
///
/// Type, required, enum, or nested property mismatches.
pub fn validate_against(schema: &Value, instance: &Value) -> Result<(), SchemaError> {
    validate_inner(schema, instance, "")
}

fn validate_inner(schema: &Value, instance: &Value, path: &str) -> Result<(), SchemaError> {
    let Some(obj) = schema.as_object() else {
        return Ok(());
    };
    if let Some(enum_values) = obj.get("enum").and_then(Value::as_array)
        && !enum_values.contains(instance)
    {
        return Err(SchemaError::at(path, "value is not in enum"));
    }
    if let Some(type_val) = obj.get("type") {
        check_type(type_val, instance, path)?;
    }
    if let Some(map) = instance.as_object() {
        if let Some(props) = obj.get("properties").and_then(Value::as_object) {
            for (key, sub) in props {
                if let Some(child) = map.get(key) {
                    let child_path = join_path(path, key);
                    validate_inner(sub, child, &child_path)?;
                }
            }
        }
        if let Some(required) = obj.get("required").and_then(Value::as_array) {
            for req in required {
                if let Some(name) = req.as_str()
                    && !map.contains_key(name)
                {
                    return Err(SchemaError::at(
                        path,
                        format!("missing required property {name}"),
                    ));
                }
            }
        }
    }
    if let Some(items) = obj.get("items")
        && let Some(arr) = instance.as_array()
    {
        for (i, item) in arr.iter().enumerate() {
            validate_inner(items, item, &format!("{path}[{i}]"))?;
        }
    }
    Ok(())
}

fn check_type(type_val: &Value, instance: &Value, path: &str) -> Result<(), SchemaError> {
    match type_val {
        Value::String(t) => check_one_type(t, instance, path),
        Value::Array(types) => {
            let ok = types.iter().any(|t| {
                t.as_str()
                    .is_some_and(|name| check_one_type(name, instance, path).is_ok())
            });
            if ok {
                Ok(())
            } else {
                Err(SchemaError::at(path, "type mismatch"))
            }
        }
        _ => Ok(()),
    }
}

fn check_one_type(type_name: &str, instance: &Value, path: &str) -> Result<(), SchemaError> {
    let ok = match type_name {
        "object" => instance.is_object(),
        "array" => instance.is_array(),
        "string" => instance.is_string(),
        "number" => instance.is_number(),
        "integer" => instance.as_i64().is_some() || instance.as_u64().is_some(),
        "boolean" => instance.is_boolean(),
        "null" => instance.is_null(),
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err(SchemaError::at(path, format!("expected {type_name}")))
    }
}

fn join_path(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

#[cfg(test)]
mod tests {
    use super::validate_against;
    use serde_json::json;

    #[test]
    fn required_and_types() {
        let schema = json!({
            "type": "object",
            "required": ["name"],
            "properties": { "name": { "type": "string" }, "n": { "type": "integer" } }
        });
        assert!(validate_against(&schema, &json!({"name": "a", "n": 1})).is_ok());
        assert!(validate_against(&schema, &json!({"n": 1})).is_err());
        assert!(validate_against(&schema, &json!({"name": 1})).is_err());
    }
}
