//! Introspection JSON → one tool per query/mutation field.

use serde_json::{Map, Value, json};

use executor_core::{PluginError, ToolDef, ToolName};

/// Extract tools from an introspection `__schema` object.
///
/// # Errors
///
/// Missing schema.
pub fn tools_from_introspection(schema: &Value) -> Result<Vec<ToolDef>, PluginError> {
    let types = type_map(schema);
    let mut tools = Vec::new();
    collect_fields(
        schema
            .get("queryType")
            .and_then(|t| t.get("name"))
            .and_then(Value::as_str),
        "query",
        &types,
        &mut tools,
    )?;
    collect_fields(
        schema
            .get("mutationType")
            .and_then(|t| t.get("name"))
            .and_then(Value::as_str),
        "mutation",
        &types,
        &mut tools,
    )?;
    Ok(tools)
}

fn type_map(schema: &Value) -> Map<String, Value> {
    let mut map = Map::new();
    if let Some(arr) = schema.get("types").and_then(Value::as_array) {
        for t in arr {
            if let Some(name) = t.get("name").and_then(Value::as_str) {
                map.insert(name.to_owned(), t.clone());
            }
        }
    }
    map
}

fn collect_fields(
    type_name: Option<&str>,
    kind: &str,
    types: &Map<String, Value>,
    tools: &mut Vec<ToolDef>,
) -> Result<(), PluginError> {
    let Some(name) = type_name else {
        return Ok(());
    };
    let Some(ty) = types.get(name) else {
        return Ok(());
    };
    let Some(fields) = ty.get("fields").and_then(Value::as_array) else {
        return Ok(());
    };
    for field in fields {
        let Some(field_name) = field.get("name").and_then(Value::as_str) else {
            continue;
        };
        if field_name.starts_with("__") {
            continue;
        }
        let args = field
            .get("args")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let (schema, var_names, var_types) = input_schema(&args);
        let description = field
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let op = operation_string(kind, field_name, &var_names, &var_types, field.get("type"));
        tools.push(ToolDef {
            name: ToolName::new(format!("{kind}.{field_name}")).map_err(PluginError::new)?,
            description,
            input_schema: Some(schema),
            output_schema: None,
            annotations: None,
            plugin_meta: Some(json!({
                "kind": kind,
                "fieldName": field_name,
                "operationString": op,
                "variableNames": var_names,
            })),
        });
    }
    Ok(())
}

fn input_schema(args: &[Value]) -> (Value, Vec<String>, Vec<String>) {
    let mut properties = Map::new();
    let mut required = Vec::new();
    let mut names = Vec::new();
    let mut types = Vec::new();
    for arg in args {
        let Some(name) = arg.get("name").and_then(Value::as_str) else {
            continue;
        };
        let ty = arg.get("type").cloned().unwrap_or(Value::Null);
        properties.insert(name.to_owned(), type_ref_schema(&ty));
        if is_non_null(&ty) {
            required.push(name.to_owned());
        }
        names.push(name.to_owned());
        types.push(format_type_ref(&ty));
    }
    let mut schema = json!({"type":"object","properties": properties});
    if !required.is_empty() {
        schema["required"] = json!(required);
    }
    (schema, names, types)
}

fn is_non_null(ty: &Value) -> bool {
    ty.get("kind").and_then(Value::as_str) == Some("NON_NULL")
}

fn type_ref_schema(ty: &Value) -> Value {
    match ty.get("kind").and_then(Value::as_str) {
        Some("NON_NULL") => ty
            .get("ofType")
            .map_or_else(|| json!({"type":"string"}), type_ref_schema),
        Some("LIST") => json!({
            "type":"array",
            "items": ty.get("ofType").map_or_else(|| json!({}), type_ref_schema)
        }),
        Some("SCALAR") => scalar_schema(ty.get("name").and_then(Value::as_str).unwrap_or("String")),
        Some("ENUM") => json!({"type":"string"}),
        _ => json!({"type":"object"}),
    }
}

fn scalar_schema(name: &str) -> Value {
    match name {
        "Int" => json!({"type":"integer"}),
        "Float" => json!({"type":"number"}),
        "Boolean" => json!({"type":"boolean"}),
        _ => json!({"type":"string"}),
    }
}

fn format_type_ref(ty: &Value) -> String {
    match ty.get("kind").and_then(Value::as_str) {
        Some("NON_NULL") => {
            format!(
                "{}!",
                ty.get("ofType")
                    .map_or_else(|| "Unknown".into(), format_type_ref)
            )
        }
        Some("LIST") => format!(
            "[{}]",
            ty.get("ofType")
                .map_or_else(|| "Unknown".into(), format_type_ref)
        ),
        _ => ty
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Unknown")
            .to_owned(),
    }
}

fn operation_string(
    kind: &str,
    field: &str,
    names: &[String],
    types: &[String],
    return_ty: Option<&Value>,
) -> String {
    let vars: Vec<String> = names
        .iter()
        .zip(types.iter())
        .map(|(n, t)| format!("${n}: {t}"))
        .collect();
    let header = if vars.is_empty() {
        format!("{kind} Op")
    } else {
        format!("{kind} Op({})", vars.join(", "))
    };
    let args = if names.is_empty() {
        String::new()
    } else {
        let inner = names
            .iter()
            .map(|n| format!("{n}: ${n}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("({inner})")
    };
    let selection = if needs_subselection(return_ty) {
        " { __typename }"
    } else {
        ""
    };
    format!("{header} {{ {field}{args}{selection} }}")
}

fn needs_subselection(ty: Option<&Value>) -> bool {
    let Some(ty) = ty else {
        return false;
    };
    match ty.get("kind").and_then(Value::as_str) {
        Some("NON_NULL" | "LIST") => needs_subselection(ty.get("ofType")),
        Some("OBJECT" | "INTERFACE" | "UNION") => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::tools_from_introspection;
    use serde_json::json;

    #[test]
    fn extracts_query_and_mutation() {
        let schema = json!({
            "queryType":{"name":"Query"},
            "mutationType":{"name":"Mutation"},
            "types":[
                {"name":"Query","fields":[
                    {"name":"user","args":[{"name":"id","type":{"kind":"NON_NULL","ofType":{"kind":"SCALAR","name":"ID"}}}],"description":"Get user","type":{"kind":"OBJECT","name":"User"}}
                ]},
                {"name":"Mutation","fields":[
                    {"name":"createUser","args":[]}
                ]}
            ]
        });
        let tools = tools_from_introspection(&schema).unwrap();
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str().to_owned()).collect();
        assert!(names.contains(&"query.user".to_owned()));
        assert!(names.contains(&"mutation.createUser".to_owned()));
        let user = tools
            .iter()
            .find(|t| t.name.as_str() == "query.user")
            .unwrap();
        let op = user.plugin_meta.as_ref().unwrap()["operationString"]
            .as_str()
            .unwrap();
        assert!(op.contains("__typename"), "{op}");
    }
}
