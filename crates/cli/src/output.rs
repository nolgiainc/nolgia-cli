use anyhow::Result;
use clap::ValueEnum;
use serde::Serialize;
use serde_json::Value;

mod path;
mod table;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum OutputMode {
    Json,
    Table,
    Value,
}

#[derive(Default)]
struct Selection {
    fields: Vec<String>,
    output: Option<OutputMode>,
}

pub struct OutputContext {
    format: OutputFormat,
    selection: Selection,
}

impl OutputContext {
    pub fn with_selection(mut self, fields: Vec<String>, output: Option<OutputMode>) -> Self {
        if !fields.is_empty() || output.is_some() {
            self.format = OutputFormat::Json;
        }
        self.selection = Selection { fields, output };
        self
    }

    pub fn format(&self) -> OutputFormat {
        self.format
    }

    pub fn is_selected(&self) -> bool {
        !self.selection.fields.is_empty()
            || matches!(
                self.selection.output,
                Some(OutputMode::Table | OutputMode::Value)
            )
    }
}

impl From<OutputFormat> for OutputContext {
    fn from(format: OutputFormat) -> Self {
        Self {
            format,
            selection: Selection::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    Text,
    Json,
}

impl OutputFormat {
    pub fn from_flags(json: bool, fields: &[String], output: Option<OutputMode>) -> Self {
        if json || !fields.is_empty() || output.is_some() {
            Self::Json
        } else {
            Self::Text
        }
    }
}

pub fn print_json<T: Serialize>(output: &OutputContext, value: &T) -> Result<()> {
    println!("{}", render(value, &output.selection)?);
    Ok(())
}

// Exit 75/65/77 reports and API error responses must retain their full object.
// Applying selection could hide recovery details or add a second field failure.
pub fn print_json_unselected<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn render<T: Serialize>(value: &T, selection: &Selection) -> Result<String> {
    let mode = selection.output.unwrap_or(if selection.fields.is_empty() {
        OutputMode::Json
    } else {
        OutputMode::Value
    });
    // Serializing a struct through Value can reorder its keys. Keep the original
    // serializer for unselected JSON so existing --json output is byte-identical.
    if selection.fields.is_empty() && mode == OutputMode::Json {
        return Ok(serde_json::to_string_pretty(value)?);
    }
    let value = serde_json::to_value(value)?;
    let selected = selection
        .fields
        .iter()
        .map(|field| path::resolve(&value, field))
        .collect::<Result<Vec<_>>>()?;
    match mode {
        OutputMode::Value => {
            if selected.is_empty() {
                Ok(bare(&value))
            } else {
                Ok(selected
                    .into_iter()
                    .map(bare)
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }
        OutputMode::Json | OutputMode::Table => {
            let selected = match selected.as_slice() {
                [] => value,
                [one] => (*one).clone(),
                many => Value::Array(many.iter().map(|value| (*value).clone()).collect()),
            };
            match mode {
                OutputMode::Json => Ok(serde_json::to_string_pretty(&selected)?),
                OutputMode::Table => Ok(table::render(&selected)),
                OutputMode::Value => unreachable!("value rendering handled above"),
            }
        }
    }
}

fn bare(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            value.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn selected(value: &Value, fields: &[&str], mode: Option<OutputMode>) -> Result<String> {
        render(
            value,
            &Selection {
                fields: fields.iter().map(|s| s.to_string()).collect(),
                output: mode,
            },
        )
    }

    #[test]
    fn paths_and_values_render_without_changing_types() {
        let value = json!({"items": [{"id": "abc", "cost": {"credits": 3}}], "ok": true});
        assert_eq!(
            selected(
                &value,
                &[".items[0].id", "items[0].cost.credits", "ok"],
                None
            )
            .unwrap(),
            "abc\n3\ntrue"
        );
        assert_eq!(
            selected(&json!([[{"name": "x"}]]), &["[0][0].name"], None).unwrap(),
            "x"
        );
        for value in [
            Value::Null,
            json!(false),
            json!(42),
            json!({"x":1}),
            json!([1, 2]),
        ] {
            assert_eq!(
                selected(&value, &[], Some(OutputMode::Value)).unwrap(),
                value.to_string()
            );
        }
        assert_eq!(
            selected(&value, &["ok"], Some(OutputMode::Json)).unwrap(),
            "true"
        );
        assert_eq!(
            selected(&value, &["ok", "items[0].id"], Some(OutputMode::Json)).unwrap(),
            "[\n  true,\n  \"abc\"\n]"
        );
    }

    #[test]
    fn missing_fields_are_atomic_and_explain_the_failed_segment() {
        let value = json!({"asset": {"id":"a", "signed_url":"https://files/a"}, "items":[1]});
        let error = selected(&value, &["asset.id", "asset.url"], None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("field \"asset.url\" not found"));
        assert!(error.contains("available at \"asset\": id, signed_url"));
        assert!(error.contains("segment \"url\""));
        let error = selected(&value, &["items[1]"], None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("past the end"));
        assert!(error.contains("array of 1 items"));
        assert!(
            selected(&value, &["asset.id.x"], None)
                .unwrap_err()
                .to_string()
                .contains("scalar")
        );
        for path in [
            "",
            ".",
            "asset..id",
            "items[-1]",
            "items[",
            "items[]",
            "items[0]id",
            "asset.",
        ] {
            assert!(selected(&value, &[path], None).is_err(), "accepted {path}");
        }
    }

    #[test]
    fn command_contexts_keep_independent_output_selections() {
        use crate::commands::CommandContext;

        let client = || {
            nolgia_client::ClientBuilder::new("http://localhost")
                .build()
                .unwrap()
        };
        let first = CommandContext::new(
            client(),
            OutputContext::from(OutputFormat::Text).with_selection(vec!["id".into()], None),
        );
        let second = CommandContext::new(
            client(),
            OutputContext::from(OutputFormat::Json)
                .with_selection(vec!["name".into()], Some(OutputMode::Json)),
        );
        let default = CommandContext::new(client(), OutputFormat::Json);
        let value = json!({"id": 7, "name": "example"});
        assert_eq!(first.format(), OutputFormat::Json);
        assert_eq!(render(&value, &first.output().selection).unwrap(), "7");
        assert_eq!(
            render(&value, &second.output().selection).unwrap(),
            "\"example\""
        );
        assert_eq!(
            render(&value, &default.output().selection).unwrap(),
            serde_json::to_string_pretty(&value).unwrap()
        );
        assert_eq!(render(&value, &first.output().selection).unwrap(), "7");
    }

    #[test]
    fn plain_json_preserves_struct_field_order() {
        #[derive(Serialize)]
        struct Record {
            z: u8,
            a: u8,
        }
        let value = Record { z: 1, a: 2 };
        for output in [None, Some(OutputMode::Json)] {
            assert_eq!(
                render(
                    &value,
                    &Selection {
                        fields: vec![],
                        output
                    }
                )
                .unwrap(),
                "{\n  \"z\": 1,\n  \"a\": 2\n}"
            );
        }
    }

    #[test]
    fn table_keeps_nested_only_and_empty_object_rows() {
        let rows = json!([{"nested": {"id": 1}}, {"nested": [2]}, {}]);
        let expected = "VALUE\n{\"nested\":{\"id\":1}}\n{\"nested\":[2]}\n{}";
        assert_eq!(table::render(&rows), expected);
        assert_eq!(table::render(&json!({"items": rows})), expected);
        assert_eq!(table::render(&json!([{}, {}])), "VALUE\n{}\n{}");
    }

    #[test]
    fn table_handles_objects_arrays_and_scalar_cells() {
        assert_eq!(
            table::render(&json!({"a": 1, "b": [2,3]})),
            "KEY  VALUE\na    1\nb    [2,3]"
        );
        assert_eq!(table::render(&json!(["a", 12, null])), "VALUE\na\n12\nnull");
        assert_eq!(
            table::render(&json!([{ "z":1 }, { "a":2, "z":{"x":3} }])),
            "z        a\n1        \n{\"x\":3}  2"
        );
        assert_eq!(table::render(&json!("bare")), "bare");
        assert_eq!(
            table::render(&json!([{ "a": {}, "b":1, "nested": [] }, { "a":2, "nested": {} }])),
            "a   b\n{}  1\n2   "
        );
    }
}
