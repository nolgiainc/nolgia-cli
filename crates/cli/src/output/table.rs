use serde_json::Value;

pub(super) fn render(value: &Value) -> String {
    match value {
        Value::Array(rows) if rows.iter().all(Value::is_object) && !rows.is_empty() => {
            let mut headers = Vec::new();
            for row in rows {
                if let Value::Object(object) = row {
                    for key in object.keys() {
                        if !headers.contains(key) {
                            headers.push(key.clone());
                        }
                    }
                }
            }
            headers.retain(|key| {
                rows.iter().any(|row| {
                    row.get(key)
                        .is_some_and(|value| !value.is_array() && !value.is_object())
                })
            });
            let mut cells = vec![headers.clone()];
            cells.extend(rows.iter().map(|row| {
                headers
                    .iter()
                    .map(|key| row.get(key).map(cell).unwrap_or_default())
                    .collect()
            }));
            align(cells)
        }
        Value::Array(rows) => {
            let mut cells = vec![vec!["VALUE".to_string()]];
            cells.extend(rows.iter().map(|value| vec![cell(value)]));
            align(cells)
        }
        // Paginated command responses expose their rows as `items`. Keep this
        // shared here so jobs/assets list do not need table-specific branches.
        Value::Object(object) if object.get("items").is_some_and(Value::is_array) => {
            render(&object["items"])
        }
        Value::Object(object) => {
            let mut cells = vec![vec!["KEY".to_string(), "VALUE".to_string()]];
            cells.extend(
                object
                    .iter()
                    .map(|(key, value)| vec![key.clone(), cell(value)]),
            );
            align(cells)
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => super::bare(value),
    }
}

fn cell(value: &Value) -> String {
    // Keep each table row on one line even when a scalar string contains tabs
    // or newlines; value mode still prints strings verbatim.
    super::bare(value)
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn align(rows: Vec<Vec<String>>) -> String {
    let columns = rows.first().map_or(0, Vec::len);
    let widths: Vec<_> = (0..columns)
        .map(|column| {
            rows.iter()
                .map(|row| row[column].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    rows.into_iter()
        .map(|row| {
            row.into_iter()
                .enumerate()
                .map(|(column, text)| {
                    if column + 1 == columns {
                        text
                    } else {
                        let padding = widths[column] - text.chars().count() + 2;
                        format!("{text}{}", " ".repeat(padding))
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
