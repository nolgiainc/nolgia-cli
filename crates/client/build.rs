use std::{
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
    Ok(serde_yaml::from_str(&serde_yaml::to_string(&value)?)?)
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
