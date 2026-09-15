//! Compact TypeScript from a JSON Schema subset (`describe.tool`).

use serde_json::Value;

/// Render a JSON Schema as a compact TypeScript type string.
#[must_use]
pub fn json_schema_to_typescript(schema: &Value) -> String {
    render(schema, 0)
}

fn render(schema: &Value, depth: usize) -> String {
    if depth > 8 {
        return "unknown".into();
    }
    let Some(obj) = schema.as_object() else {
        return "unknown".into();
    };
    if let Some(values) = obj.get("enum").and_then(Value::as_array) {
        let parts: Vec<String> = values.iter().map(literal).collect();
        if parts.is_empty() {
            return "never".into();
        }
        return parts.join(" | ");
    }
    if let Some(consts) = obj.get("const") {
        return literal(consts);
    }
    match obj.get("type").and_then(Value::as_str).unwrap_or("") {
        "string" => "string".into(),
        "number" | "integer" => "number".into(),
        "boolean" => "boolean".into(),
        "null" => "null".into(),
        "array" => {
            let inner = obj
                .get("items")
                .map_or_else(|| "unknown".into(), |i| render(i, depth + 1));
            format!("{inner}[]")
        }
        "object" | "" => render_object(obj, depth),
        _ => "unknown".into(),
    }
}

fn render_object(obj: &serde_json::Map<String, Value>, depth: usize) -> String {
    let Some(props) = obj.get("properties").and_then(Value::as_object) else {
        if obj.get("additionalProperties").is_some() {
            return "{ [k: string]: unknown }".into();
        }
        return "Record<string, unknown>".into();
    };
    if props.is_empty() {
        return "Record<string, unknown>".into();
    }
    let required: Vec<&str> = obj
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut fields = Vec::new();
    for (key, sub) in props {
        let opt = if required.iter().any(|r| *r == key) {
            ""
        } else {
            "?"
        };
        fields.push(format!("{key}{opt}: {}", render(sub, depth + 1)));
    }
    format!("{{ {} }}", fields.join("; "))
}

fn literal(value: &Value) -> String {
    match value {
        Value::String(s) => format!("\"{}\"", s.replace('"', "\\\"")),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".into(),
        _ => "unknown".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::json_schema_to_typescript;
    use serde_json::json;

    #[test]
    fn object_with_required_and_optional() {
        let ts = json_schema_to_typescript(&json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {"type": "string"},
                "n": {"type": "integer"}
            }
        }));
        assert!(ts.contains("name: string"));
        assert!(ts.contains("n?: number"));
    }
}
