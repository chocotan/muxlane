use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ElicitationField {
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) description: Option<String>,
    pub(crate) required: bool,
    pub(crate) default: Option<Value>,
    pub(crate) min_length: Option<usize>,
    pub(crate) max_length: Option<usize>,
    pub(crate) pattern: Option<String>,
    pub(crate) minimum: Option<f64>,
    pub(crate) maximum: Option<f64>,
    pub(crate) kind: ElicitationFieldKind,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ElicitationFieldKind {
    String { choices: Vec<String> },
    Number,
    Integer,
    Boolean,
    MultiSelect { choices: Vec<String> },
    Unsupported(String),
}

pub(crate) fn fields_from_schema(schema: &Value) -> Vec<ElicitationField> {
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<std::collections::HashSet<_>>();
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    properties
        .iter()
        .map(|(name, schema)| {
            let type_name = schema
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let enum_choices = choices(schema);
            let kind = match type_name {
                "string" => ElicitationFieldKind::String {
                    choices: enum_choices,
                },
                "number" => ElicitationFieldKind::Number,
                "integer" => ElicitationFieldKind::Integer,
                "boolean" => ElicitationFieldKind::Boolean,
                "array" => ElicitationFieldKind::MultiSelect {
                    choices: schema.get("items").map(choices).unwrap_or_default(),
                },
                other => ElicitationFieldKind::Unsupported(other.to_string()),
            };
            ElicitationField {
                name: name.clone(),
                title: schema
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or(name)
                    .to_string(),
                description: schema
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                required: required.contains(name.as_str()),
                default: schema.get("default").cloned(),
                min_length: schema
                    .get("minLength")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok()),
                max_length: schema
                    .get("maxLength")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok()),
                pattern: schema
                    .get("pattern")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                minimum: schema.get("minimum").and_then(Value::as_f64),
                maximum: schema.get("maximum").and_then(Value::as_f64),
                kind,
            }
        })
        .collect()
}

fn choices(schema: &Value) -> Vec<String> {
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
    }
    schema
        .get("oneOf")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|choice| {
            choice
                .get("const")
                .or_else(|| choice.get("value"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

pub(crate) fn parse_field_value(
    field: &ElicitationField,
    text: &str,
    selected: Option<&Value>,
) -> Result<Option<Value>, String> {
    if let Some(value) = selected {
        return Ok(Some(value.clone()));
    }
    let text = text.trim();
    if text.is_empty() {
        if let Some(default) = &field.default {
            return Ok(Some(default.clone()));
        }
        return if field.required {
            Err(format!("{} is required", field.title))
        } else {
            Ok(None)
        };
    }
    match &field.kind {
        ElicitationFieldKind::String { .. } => {
            let length = text.chars().count();
            if field.min_length.is_some_and(|minimum| length < minimum) {
                return Err(format!("{} is too short", field.title));
            }
            if field.max_length.is_some_and(|maximum| length > maximum) {
                return Err(format!("{} is too long", field.title));
            }
            if let Some(pattern) = &field.pattern {
                let pattern = regex::Regex::new(pattern)
                    .map_err(|_| format!("{} has an invalid validation pattern", field.title))?;
                if !pattern.is_match(text) {
                    return Err(format!("{} has an invalid format", field.title));
                }
            }
            Ok(Some(Value::String(text.to_string())))
        }
        ElicitationFieldKind::Number => {
            let value = text
                .parse::<f64>()
                .map_err(|_| format!("{} must be a number", field.title))?;
            validate_numeric_range(field, value)?;
            serde_json::Number::from_f64(value)
                .map(Value::Number)
                .map(Some)
                .ok_or_else(|| format!("{} must be a finite number", field.title))
        }
        ElicitationFieldKind::Integer => {
            let value = text
                .parse::<i64>()
                .map_err(|_| format!("{} must be an integer", field.title))?;
            validate_numeric_range(field, value as f64)?;
            Ok(Some(Value::Number(value.into())))
        }
        ElicitationFieldKind::Boolean
        | ElicitationFieldKind::MultiSelect { .. }
        | ElicitationFieldKind::Unsupported(_) => Err(format!("{} needs a selection", field.title)),
    }
}

fn validate_numeric_range(field: &ElicitationField, value: f64) -> Result<(), String> {
    if field.minimum.is_some_and(|minimum| value < minimum) {
        return Err(format!("{} is below the minimum", field.title));
    }
    if field.maximum.is_some_and(|maximum| value > maximum) {
        return Err(format!("{} is above the maximum", field.title));
    }
    Ok(())
}

pub(crate) fn valid_elicitation_url(value: &str) -> bool {
    url::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

pub(crate) fn values_map(
    fields: &[ElicitationField],
    text_values: &std::collections::HashMap<String, String>,
    selected_values: &std::collections::HashMap<String, Value>,
) -> Result<Map<String, Value>, String> {
    let mut result = Map::new();
    for field in fields {
        if let Some(value) = parse_field_value(
            field,
            text_values
                .get(&field.name)
                .map(String::as_str)
                .unwrap_or_default(),
            selected_values.get(&field.name),
        )? {
            result.insert(field.name.clone(), value);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_supported_field_kinds_and_required_values() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {"type": "string", "title": "Name"},
                "count": {"type": "integer"},
                "enabled": {"type": "boolean", "default": true},
                "mode": {"type": "string", "enum": ["fast", "safe"]},
                "tags": {"type": "array", "items": {"type": "string", "enum": ["a", "b"]}}
            }
        });
        let fields = fields_from_schema(&schema);
        assert_eq!(fields.len(), 5);
        assert!(fields
            .iter()
            .any(|field| field.name == "name" && field.required));
        assert!(matches!(
            fields
                .iter()
                .find(|field| field.name == "tags")
                .unwrap()
                .kind,
            ElicitationFieldKind::MultiSelect { .. }
        ));
    }

    #[test]
    fn validates_urls_and_numeric_fields() {
        assert!(valid_elicitation_url("https://example.com/login"));
        assert!(!valid_elicitation_url("javascript:alert(1)"));
        let field = ElicitationField {
            name: "count".into(),
            title: "Count".into(),
            description: None,
            required: true,
            default: None,
            min_length: None,
            max_length: None,
            pattern: None,
            minimum: Some(1.0),
            maximum: Some(100.0),
            kind: ElicitationFieldKind::Integer,
        };
        assert_eq!(
            parse_field_value(&field, "42", None).unwrap(),
            Some(serde_json::json!(42))
        );
        assert!(parse_field_value(&field, "no", None).is_err());
    }
}
