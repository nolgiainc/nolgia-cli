use std::{
    collections::BTreeSet,
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use openapiv3::OpenAPI;
use progenitor::{GenerationSettings, Generator, InterfaceStyle};
use regex::Regex;
use serde_yaml::Value;

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let version_file = manifest_dir.join("openapi-version.toml");
    let local_spec = manifest_dir.join("../../../nolgia-api/api/openapi.yaml");
    let vendored_spec = manifest_dir.join("openapi.yaml");
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);

    println!("cargo:rerun-if-changed={}", version_file.display());
    println!("cargo:rerun-if-changed={}", local_spec.display());
    println!("cargo:rerun-if-changed={}", vendored_spec.display());
    println!("cargo:rerun-if-env-changed=NOLGIA_OPENAPI_RELEASE_URL");
    println!("cargo:rerun-if-env-changed=NOLGIA_USE_SIBLING_SPEC");

    let spec = load_spec(&local_spec, &vendored_spec, &version_file)?;

    let mut settings = GenerationSettings::default();
    settings.with_interface(InterfaceStyle::Builder);

    let mut generator = Generator::new(&settings);
    let tokens = generator.generate_tokens(&spec)?;
    let ast = syn::parse2(tokens)?;
    let code = prettyplease::unparse(&ast);

    fs::write(out_dir.join("codegen.rs"), code)?;
    Ok(())
}

fn load_spec(
    local_spec: &Path,
    vendored_spec: &Path,
    version_file: &Path,
) -> Result<OpenAPI, Box<dyn Error>> {
    // Spec source precedence:
    //   1. The sibling nolgia-api checkout — LOCAL DEV CONVENIENCE ONLY, and
    //      only when opted in via NOLGIA_USE_SIBLING_SPEC. CI must never prefer
    //      the sibling: a stale sibling checkout could silently mask spec drift
    //      that the vendored copy (which the spec-check job gates) would catch.
    //   2. The vendored snapshot (crates/client/openapi.yaml) — the default,
    //      and the only source used in CI/release builds.
    //   3. The release asset download (release profile, no vendored copy).
    let use_sibling =
        matches!(env::var("NOLGIA_USE_SIBLING_SPEC").as_deref(), Ok("1")) && local_spec.exists();
    let raw_text = if use_sibling {
        fs::read_to_string(local_spec)?
    } else if vendored_spec.exists() {
        fs::read_to_string(vendored_spec)?
    } else if is_release_profile() {
        let version = read_spec_version(version_file)?;
        let url = env::var("NOLGIA_OPENAPI_RELEASE_URL").unwrap_or_else(|_| {
            format!(
                "https://github.com/nolgiainc/nolgia-api/releases/download/v{version}/openapi.yaml"
            )
        });
        let response = reqwest::blocking::get(url)?;
        let response = response.error_for_status()?;
        response.text()?
    } else {
        return Err(format!(
            "no OpenAPI spec found at {} or {}",
            local_spec.display(),
            vendored_spec.display()
        )
        .into());
    };

    let mut value: Value = serde_yaml::from_str(&sanitize_openapi_text(&raw_text))?;
    strip_non_success_responses(&mut value);
    strip_additional_properties_false(&mut value);
    unmaterialize_server_side_defaults(&mut value);
    relax_request_model_selectors(&mut value);
    relax_response_only_enums(&mut value);
    Ok(serde_yaml::from_str(&serde_yaml::to_string(&value)?)?)
}

/// Request-body fields whose `default:` describes what the **server** does when
/// the field is absent, and which must therefore never be materialized onto the
/// wire by the client.
///
/// Progenitor turns a non-nullable property with a `default:` into a plain
/// (non-`Option`) field carrying `#[serde(default = ...)]` and **no**
/// `skip_serializing_if`, so the value is serialized on every request whether
/// or not the caller asked for it. For most defaults that is harmless — the
/// value sent equals the value the server would have chosen anyway
/// (`GenerateImageRequest.num_images: 1` is unconditionally correct, so it is
/// deliberately not listed here).
///
/// It is *not* harmless when the field's real default is dynamic. On
/// `GenerateVideoRequest`, `duration_seconds` defaults to the sum of `shots[]`
/// when shots are supplied; the static `default: 5` in the spec is only correct
/// for the shot-less case. Materializing it meant every multi-shot submit
/// carried `duration_seconds: 5` alongside shots summing to something else, and
/// the API rejected the contradiction:
///
/// ```text
/// 400 duration_seconds (5) must equal the sum of shot durations (10) — or omit it
/// ```
///
/// That 400 hit every `--shot` caller (NOL-342) and made the `short-film`
/// preset unrunnable, because there was no way for the CLI to express "absent".
/// Dropping the `default` and marking the property nullable makes progenitor
/// emit `Option<T>` with `skip_serializing_if`, so an unset field is genuinely
/// omitted and the server applies its own default — static or dynamic.
///
/// This rewrites the in-memory spec at codegen time only; the vendored
/// `openapi.yaml` stays byte-identical to the published canonical spec, which
/// is what the `spec-check` CI job diffs.
const SERVER_SIDE_DEFAULT_FIELDS: &[(&str, &str)] = &[("GenerateVideoRequest", "duration_seconds")];

fn unmaterialize_server_side_defaults(value: &mut Value) {
    let Some(schemas) = value
        .get_mut("components")
        .and_then(|c| c.get_mut("schemas"))
        .and_then(Value::as_mapping_mut)
    else {
        return;
    };

    for (schema_name, property_name) in SERVER_SIDE_DEFAULT_FIELDS {
        let Some(property) = schemas
            .get_mut(Value::from(*schema_name))
            .and_then(|s| s.get_mut("properties"))
            .and_then(|p| p.get_mut(Value::from(*property_name)))
            .and_then(Value::as_mapping_mut)
        else {
            // The spec moved on (field renamed or already nullable upstream).
            // Not fatal: the generated client simply keeps whatever shape the
            // current spec implies.
            continue;
        };
        property.remove(Value::from("default"));
        property.insert(Value::from("nullable"), Value::from(true));
    }
}

/// Request-body `model` selectors whose `$ref` points at a closed
/// `{Image,Video,Audio}Model` string enum, and which must be relaxed to a
/// plain `type: string` so the client forwards whatever model the caller
/// names and lets the **API** decide whether it exists.
///
/// This is the request-side twin of [`relax_response_only_enums`], and it
/// exists for the same reason: the model list is an *additive* contract that
/// the API extends within a version, and a client that hard-codes it as a
/// closed set rejects every model added after it was built. A generated enum
/// keeps `--model` in lockstep with the vendored spec, but the vendored spec
/// only reaches users through a **release** — so a model the API already
/// serves is rejected by every binary cut before its re-vendor. That is
/// exactly NOL-439: BFL FLUX 3 (`flux-3-video`) went live in the API and in
/// the vendored spec, yet the last released CLI (v0.2.18, cut before the
/// re-vendor) died at argument parsing with
///
/// ```text
/// error: invalid value 'flux-3-video' for '--model <MODEL>': invalid value
/// ```
///
/// while the raw `POST /generate/video {model: "flux-3-video"}` accepted it.
/// Re-vendoring and re-releasing fixes the symptom once; relaxing the selector
/// fixes the whole class — a new model works on an old binary, and no CLI
/// release is needed to adopt one.
///
/// The `model` selector is deliberately treated differently from the other
/// request enums the sibling function leaves strict (e.g.
/// `GenerateImageRequest.aspect_ratio`, whose strictness NOL-345 relies on to
/// name every accepted ratio on a miss). Those have a small, slow vocabulary
/// and are additionally checked per-model against `GET /models`; `model` is
/// the primary, frequently-extended selector with no second live check, so the
/// closed enum *is* the only gate and it is the one that rots. A model the
/// server rejects still fails — just legibly, from the API's own RFC 7807
/// response through `submit_error`, which can name the real catalog — instead
/// of from a stale client-side list.
///
/// Only the *use* at these properties is rewritten; the `{Image,Video,Audio}
/// Model` schemas stay defined, so the generated enum types (and their
/// `Display`/`FromStr`) remain available. As with every transform here this
/// edits the in-memory spec at codegen time only — the vendored `openapi.yaml`
/// stays byte-identical to the published contract the `spec-check` CI job
/// diffs.
const MODEL_SELECTOR_FIELDS: &[(&str, &str)] = &[
    ("GenerateImageRequest", "model"),
    ("GenerateVideoRequest", "model"),
    ("GenerateAudioRequest", "model"),
];

fn relax_request_model_selectors(value: &mut Value) {
    let Some(schemas) = value
        .get_mut("components")
        .and_then(|c| c.get_mut("schemas"))
        .and_then(Value::as_mapping_mut)
    else {
        return;
    };

    for (schema_name, property_name) in MODEL_SELECTOR_FIELDS {
        let Some(property) = schemas
            .get_mut(Value::from(*schema_name))
            .and_then(|s| s.get_mut("properties"))
            .and_then(|p| p.get_mut(Value::from(*property_name)))
            .and_then(Value::as_mapping_mut)
        else {
            // The spec moved on (schema or field renamed). Not fatal: the
            // generated client keeps whatever shape the current spec implies.
            continue;
        };
        // Drop the `$ref` to the closed enum and pin the property to a plain
        // string, which progenitor emits as `String` — deserializing and,
        // crucially, *sending* any value the caller supplies.
        property.clear();
        property.insert(Value::from("type"), Value::from("string"));
    }
}

/// Remove every `additionalProperties: false` from the spec before codegen.
///
/// Progenitor translates `additionalProperties: false` into
/// `#[serde(deny_unknown_fields)]`, which makes released binaries reject any
/// response containing a field their vendored spec predates. That is exactly
/// how api#158's additive `image` capabilities field broke `models list` in
/// every released CLI (NOL-48). The API only ever evolves additively within a
/// version, so generated response types must tolerate unknown fields; the
/// strictness stays server-side (the API still validates requests against the
/// canonical spec). Explicit schema-valued or `true` `additionalProperties`
/// are preserved — they change the generated type (maps), not strictness.
fn strip_additional_properties_false(value: &mut Value) {
    match value {
        Value::Mapping(map) => {
            map.retain(|key, val| {
                !(key.as_str() == Some("additionalProperties") && val.as_bool() == Some(false))
            });
            for val in map.values_mut() {
                strip_additional_properties_false(val);
            }
        }
        Value::Sequence(seq) => {
            for val in seq.iter_mut() {
                strip_additional_properties_false(val);
            }
        }
        _ => {}
    }
}

/// Relax closed string enums to plain strings wherever they are only ever
/// *received*, so a value the vendored spec predates cannot fail a response.
///
/// `strip_additional_properties_false` above made responses tolerate unknown
/// *fields* after api#158's additive `image` capabilities broke `models list`
/// (NOL-48). It did nothing for unknown *values*, so the identical failure
/// recurred twice more: NOL-69 on the same field, then NOL-351 when NOL-208
/// added the `3:1` image aspect ratio and every binary built before that
/// re-vendor died on the whole catalog with
///
/// ```text
/// unknown variant `3:1`, expected one of `16:9`, `9:16`, `1:1`, …
/// ```
///
/// That is a startup failure, not a submission failure: `models list`,
/// `models get`, `gen video --cost-only` and every capability precheck fetch
/// `GET /models` first, so one unrecognised ratio took out jobs that never
/// mentioned an aspect ratio. The API is right — it only ever adds values
/// within a version — and a client that treats an additive contract as a
/// closed set will keep breaking on every addition until someone re-vendors
/// and re-releases. Three occurrences is enough evidence that re-vendoring is
/// not the fix.
///
/// So: a value we do not recognise becomes an option we cannot *offer*,
/// rather than a crash. Progenitor turns a `type: string` with no `enum` into
/// a plain `String`, which deserializes anything and — unlike a catch-all
/// variant — preserves the value, so a newly added ratio is listed correctly
/// by `models list` on a CLI that predates it.
///
/// **Requests stay strict.** Only schemas reachable from responses and *not*
/// from any request body or query parameter are relaxed, and the rewrite is
/// applied at the point of use rather than to the shared definition. So
/// `ImageCapabilities.aspect_ratios` (received) becomes `Vec<String>` while
/// `GenerateImageRequest.aspect_ratio` (sent) keeps the generated enum, and
/// with it NOL-345's `--aspect-ratio` validation that names every accepted
/// value on a miss. Sending a ratio the server rejects should still fail
/// early and legibly; being *told* about one should not fail at all.
///
/// Rewrites the in-memory spec at codegen time only — the vendored
/// `openapi.yaml` stays byte-identical to the published contract, which is
/// what the `spec-check` CI job diffs.
fn relax_response_only_enums(value: &mut Value) {
    let string_enums = string_enum_schema_names(value);
    if string_enums.is_empty() {
        return;
    }

    let request_reachable = reachable_schemas(value, SchemaRole::Request);
    let response_reachable = reachable_schemas(value, SchemaRole::Response);

    let relaxable: Vec<String> = response_reachable
        .difference(&request_reachable)
        .cloned()
        .collect();

    let Some(schemas) = value
        .get_mut("components")
        .and_then(|c| c.get_mut("schemas"))
        .and_then(Value::as_mapping_mut)
    else {
        return;
    };

    for name in relaxable {
        if let Some(schema) = schemas.get_mut(Value::from(name.as_str())) {
            relax_enum_uses(schema, &string_enums);
        }
    }
}

/// Names of every `components.schemas` entry that is a closed set of strings.
fn string_enum_schema_names(value: &Value) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let Some(schemas) = value
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(Value::as_mapping)
    else {
        return names;
    };

    for (name, schema) in schemas {
        let (Some(name), Some(schema)) = (name.as_str(), schema.as_mapping()) else {
            continue;
        };
        let is_string = schema.get(Value::from("type")).and_then(Value::as_str) == Some("string");
        if is_string && schema.contains_key(Value::from("enum")) {
            names.insert(name.to_string());
        }
    }

    names
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SchemaRole {
    /// Anything the client *sends*: request bodies and query/path parameters.
    Request,
    /// Anything the client *receives*.
    Response,
}

/// Every schema reachable from the operations in `role` position, following
/// `$ref`s transitively.
fn reachable_schemas(value: &Value, role: SchemaRole) -> BTreeSet<String> {
    let mut seeds = BTreeSet::new();

    if let Some(paths) = value.get("paths").and_then(Value::as_mapping) {
        for path_item in paths.values() {
            let Some(path_item) = path_item.as_mapping() else {
                continue;
            };

            // Parameters may be declared once for the whole path item.
            if role == SchemaRole::Request
                && let Some(params) = path_item.get(Value::from("parameters"))
            {
                collect_schema_refs(params, &mut seeds);
            }

            for (field, operation) in path_item {
                let Some(operation) = operation.as_mapping() else {
                    continue;
                };
                if field.as_str() == Some("parameters") {
                    continue;
                }

                match role {
                    SchemaRole::Request => {
                        for key in ["requestBody", "parameters"] {
                            if let Some(node) = operation.get(Value::from(key)) {
                                collect_schema_refs(node, &mut seeds);
                            }
                        }
                    }
                    SchemaRole::Response => {
                        if let Some(node) = operation.get(Value::from("responses")) {
                            collect_schema_refs(node, &mut seeds);
                        }
                    }
                }
            }
        }
    }

    let schemas = value
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(Value::as_mapping);

    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut stack: Vec<String> = seeds.into_iter().collect();
    while let Some(name) = stack.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(schema) = schemas.and_then(|s| s.get(Value::from(name.as_str()))) else {
            continue;
        };
        let mut nested = BTreeSet::new();
        collect_schema_refs(schema, &mut nested);
        stack.extend(nested);
    }

    seen
}

/// Collect the target names of every `$ref: '#/components/schemas/…'` under
/// `node`.
fn collect_schema_refs(node: &Value, out: &mut BTreeSet<String>) {
    match node {
        Value::Mapping(map) => {
            for (key, val) in map {
                if key.as_str() == Some("$ref")
                    && let Some(target) = val.as_str().and_then(schema_ref_name)
                {
                    out.insert(target.to_string());
                }
                collect_schema_refs(val, out);
            }
        }
        Value::Sequence(seq) => {
            for val in seq {
                collect_schema_refs(val, out);
            }
        }
        _ => {}
    }
}

fn schema_ref_name(reference: &str) -> Option<&str> {
    reference.strip_prefix("#/components/schemas/")
}

/// Replace, in place, every *use* of a closed string enum under `node` with a
/// plain `type: string`, and drop inline `enum` constraints on string
/// properties. Shared definitions are untouched — only this schema's view of
/// them changes, which is what keeps request types strict.
fn relax_enum_uses(node: &mut Value, string_enums: &BTreeSet<String>) {
    match node {
        Value::Mapping(map) => {
            let refers_to_closed_enum = map
                .get(Value::from("$ref"))
                .and_then(Value::as_str)
                .and_then(schema_ref_name)
                .is_some_and(|target| string_enums.contains(target));

            if refers_to_closed_enum {
                map.clear();
                map.insert(Value::from("type"), Value::from("string"));
                return;
            }

            let is_inline_string_enum = map.get(Value::from("type")).and_then(Value::as_str)
                == Some("string")
                && map.contains_key(Value::from("enum"));
            if is_inline_string_enum {
                map.remove(Value::from("enum"));
            }

            for val in map.values_mut() {
                relax_enum_uses(val, string_enums);
            }
        }
        Value::Sequence(seq) => {
            for val in seq.iter_mut() {
                relax_enum_uses(val, string_enums);
            }
        }
        _ => {}
    }
}

fn is_release_profile() -> bool {
    matches!(env::var("PROFILE").as_deref(), Ok("release"))
}

fn read_spec_version(version_file: &Path) -> Result<String, Box<dyn Error>> {
    let contents = fs::read_to_string(version_file)?;
    for line in contents.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("spec_version = ") {
            return Ok(rest.trim_matches('"').to_string());
        }
    }

    Err(format!("missing spec_version in {}", version_file.display()).into())
}

fn sanitize_openapi_text(input: &str) -> String {
    let mut text = input.replace("openapi: 3.1.0", "openapi: 3.0.3");
    text = text.replace("openapi: '3.1.0'", "openapi: 3.0.3");

    let nullable_type =
        Regex::new(r"(?m)^(?P<indent>\s*)type:\s+\[(?P<ty>[^,\]]+),\s*'null'\]\s*$")
            .expect("valid regex");
    text = nullable_type
        .replace_all(&text, "$indenttype: $ty\n$indentnullable: true")
        .into_owned();

    text = text.replace(
        "        asset:\n          oneOf:\n            - $ref: '#/components/schemas/Asset'\n            - type: 'null'\n",
        "        asset:\n          allOf:\n            - $ref: '#/components/schemas/Asset'\n          nullable: true\n",
    );
    text = text.replace(
        "        error:\n          oneOf:\n            - $ref: '#/components/schemas/Error'\n            - type: 'null'\n",
        "        error:\n          allOf:\n            - $ref: '#/components/schemas/Error'\n          nullable: true\n",
    );

    text
}

fn strip_non_success_responses(value: &mut Value) {
    let Some(paths) = value.get_mut("paths").and_then(Value::as_mapping_mut) else {
        return;
    };

    for path_item in paths.values_mut() {
        let Some(path_item_map) = path_item.as_mapping_mut() else {
            continue;
        };

        for method_value in path_item_map.values_mut() {
            let Some(method_map) = method_value.as_mapping_mut() else {
                continue;
            };

            let Some(responses) = method_map
                .get_mut(Value::from("responses"))
                .and_then(Value::as_mapping_mut)
            else {
                continue;
            };

            responses.retain(|status, _| is_success_status(status));
        }
    }
}

fn is_success_status(status: &Value) -> bool {
    let Some(status) = status.as_str() else {
        return false;
    };

    status.starts_with("2")
}
