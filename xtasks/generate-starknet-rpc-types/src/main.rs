use std::collections::{HashMap, HashSet};
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use heck::{ToPascalCase, ToSnakeCase};
use serde::Deserialize;
use serde_json::{Map, Value};

// Define structure to parse OpenRPC JSON schema
#[derive(Debug, Deserialize, Clone)]
#[serde(rename = "camelCase")]
struct OpenRPCSchema {
    openrpc: String,
    info: Info,
    #[serde(default)]
    servers: Vec<Method>,
    methods: Vec<Method>,
    components: Components,
    #[serde(default)]
    external_docs: Option<ExternalDocs>,
}

#[derive(Debug, Deserialize, Clone)]
struct ExternalDocs {
    url: String,
    description: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct Info {
    title: String,
    version: String,
    description: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct Method {
    #[serde(default)]
    name: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    params: Vec<Parameter>,
    #[serde(default)]
    result: ResultDefinition,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct Parameter {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    schema: Value,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct ResultDefinition {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    schema: Value,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct Components {
    #[serde(default)]
    schemas: HashMap<String, Value>,
    #[serde(default)]
    contentDescriptors: HashMap<String, Value>,
    #[serde(default)]
    errors: HashMap<String, Value>,
}

// Helper structure to organize type definitions and imports
#[derive(Debug, Default)]
struct TypeGenerator {
    types: HashMap<String, String>,
    enums: HashMap<String, String>,
    imported_types: HashSet<String>,
    processed_refs: HashSet<String>,
    base_dir: PathBuf,
    current_file: PathBuf,
    schema_cache: HashMap<String, OpenRPCSchema>,
}

impl TypeGenerator {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir, ..Default::default() }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <path-to-starknet-specs>", args[0]);
        process::exit(1);
    }

    let base_dir = PathBuf::from(&args[1]);

    // Starting with the main API file
    let main_api_file = base_dir.join("api/starknet_api_openrpc.json");

    // Process all schema files in the directory
    let mut generator = TypeGenerator::new(base_dir.clone());

    // Process all API files
    process_directory(&base_dir.join("api"), &mut generator)?;
    process_directory(&base_dir.join("wallet-api"), &mut generator)?;

    // Generate the Rust code
    generate_rust_code(&generator)?;

    Ok(())
}

fn process_directory(dir: &Path, generator: &mut TypeGenerator) -> Result<(), Box<dyn Error>> {
    if !dir.exists() || !dir.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() && path.extension().map_or(false, |ext| ext == "json") {
            println!("Processing file: {}", path.display());
            generator.current_file = path.clone();
            process_schema_file(&path, generator)?;
        }
    }

    Ok(())
}

fn process_schema_file(
    file_path: &Path,
    generator: &mut TypeGenerator,
) -> Result<(), Box<dyn Error>> {
    let content = fs::read_to_string(file_path)?;
    let schema: OpenRPCSchema = serde_json::from_str(&content)?;

    // Cache the schema
    let relative_path = file_path.strip_prefix(&generator.base_dir)?.to_string_lossy().to_string();
    generator.schema_cache.insert(relative_path.clone(), schema.clone());

    // Process components schemas
    for (name, schema_value) in &schema.components.schemas {
        process_type(name, schema_value, generator)?;
    }

    // Process method parameters and results
    for method in &schema.methods {
        // Generate a struct for method parameters
        if !method.params.is_empty() {
            let params_name = format!("{}Params", method.name.to_pascal_case());
            let mut params_struct = String::new();
            params_struct.push_str(&format!(
                "#[derive(Debug, Clone, Serialize, Deserialize)]\npub struct {} {{\n",
                params_name
            ));

            for param in &method.params {
                let field_name = sanitize_field_name(&param.name);
                let (field_type, _) = resolve_type(&param.schema, generator)?;
                let required = param.required;

                if !required {
                    params_struct.push_str(&format!("    #[serde(skip_serializing_if = \"Option::is_none\")]\n    pub {}: Option<{}>,\n", field_name, field_type));
                } else {
                    params_struct.push_str(&format!("    pub {}: {},\n", field_name, field_type));
                }
            }

            params_struct.push_str("}\n\n");
            generator.types.insert(params_name, params_struct);
        }

        // Generate a struct for method result
        if !method.result.name.is_empty() {
            let result_name = format!("{}Result", method.name.to_pascal_case());
            process_type(&result_name, &method.result.schema, generator)?;
        }
    }

    Ok(())
}

fn process_type<'a>(
    name: &str,
    schema_value: &'a Value,
    generator: &'a mut TypeGenerator,
) -> Result<(), Box<dyn Error>> {
    // Skip if we've already processed this type
    if generator.types.contains_key(name) || generator.enums.contains_key(name) {
        return Ok(());
    }

    match schema_value {
        Value::Object(obj) => {
            // Check if it's a reference
            if let Some(ref_value) = obj.get("$ref") {
                if let Value::String(ref_str) = ref_value {
                    resolve_reference(ref_str, generator)?;
                    return Ok(());
                }
            }

            // Check if it's an enum
            if let Some(Value::Array(enum_values)) = obj.get("enum") {
                if !enum_values.is_empty() {
                    generate_enum(name, enum_values, generator)?;
                    return Ok(());
                }
            }

            // Check if it's a oneOf or anyOf (generate enum)
            for key in ["oneOf", "anyOf"] {
                if let Some(Value::Array(variants)) = obj.get(key) {
                    if !variants.is_empty() {
                        generate_oneOf_enum(name, variants, generator)?;
                        return Ok(());
                    }
                }
            }

            // Handle allOf (combine properties)
            if let Some(Value::Array(all_of)) = obj.get("allOf") {
                generate_allOf_struct(name, all_of, generator)?;
                return Ok(());
            }

            // Default: treat as a struct with properties
            if let Some(Value::Object(properties)) = obj.get("properties") {
                generate_struct(name, properties, obj.get("required"), generator)?;
                return Ok(());
            }

            // Handle simple types
            if let Some(Value::String(type_str)) = obj.get("type") {
                let rust_type = map_json_type_to_rust(type_str, obj, generator)?;
                let type_def = format!("pub type {} = {};\n\n", name.to_pascal_case(), rust_type);
                generator.types.insert(name.to_string(), type_def);
                return Ok(());
            }
        }
        Value::String(type_str) => {
            // Handle direct string type
            let rust_type = map_simple_type_to_rust(type_str);
            let type_def = format!("pub type {} = {};\n\n", name.to_pascal_case(), rust_type);
            generator.types.insert(name.to_string(), type_def);
            return Ok(());
        }
        _ => {}
    }

    // Default fallback
    let type_def = format!("pub type {} = serde_json::Value;\n\n", name.to_pascal_case());
    generator.types.insert(name.to_string(), type_def);

    Ok(())
}

fn resolve_reference(
    ref_str: &str,
    generator: &mut TypeGenerator,
) -> Result<(String, bool), Box<dyn Error>> {
    // Check if we've already processed this reference
    if generator.processed_refs.contains(ref_str) {
        let last_segment = ref_str.split('/').last().unwrap_or(ref_str);
        return Ok((last_segment.to_pascal_case(), false));
    }

    generator.processed_refs.insert(ref_str.to_string());

    // Handle external file references
    if ref_str.starts_with("./") || ref_str.starts_with("../") || !ref_str.starts_with("#") {
        let parts: Vec<&str> = ref_str.split('#').collect();

        let file_path = if parts[0].is_empty() || parts[0] == "" {
            // Reference within the same file
            generator.current_file.clone()
        } else {
            // External file reference
            let current_dir = generator.current_file.parent().unwrap_or(Path::new(""));
            current_dir.join(parts[0])
        };

        // Load schema if not already cached
        let relative_path =
            file_path.strip_prefix(&generator.base_dir)?.to_string_lossy().to_string();

        if !generator.schema_cache.contains_key(&relative_path) {
            if file_path.exists() {
                let content = fs::read_to_string(&file_path)?;
                let schema: OpenRPCSchema = serde_json::from_str(&content)?;
                generator.schema_cache.insert(relative_path.clone(), schema);
            }
        }

        if parts.len() > 1 {
            // Process the reference path within the schema
            let path_parts: Vec<&str> = parts[1].split('/').filter(|p| !p.is_empty()).collect();

            if let Some(schema) = generator.schema_cache.get(&relative_path) {
                let mut current_value = Value::Object(Map::new());

                // Navigate through components/schemas/TYPE
                if path_parts.len() >= 3
                    && path_parts[0] == "components"
                    && path_parts[1] == "schemas"
                {
                    if let Some(schema_value) = schema.components.schemas.get(path_parts[2]) {
                        let type_name = path_parts[2];
                        process_type(type_name, schema_value, generator)?;
                        return Ok((type_name.to_pascal_case(), false));
                    }
                }
            }
        }
    } else {
        // Local reference within the same file
        let path_parts: Vec<&str> = ref_str[1..].split('/').collect();

        if path_parts.len() >= 3 && path_parts[0] == "components" && path_parts[1] == "schemas" {
            let type_name = path_parts[2];

            // Look up in the current schema
            if let Some(schema) = generator.schema_cache.get(
                &generator
                    .current_file
                    .strip_prefix(&generator.base_dir)?
                    .to_string_lossy()
                    .to_string(),
            ) {
                if let Some(schema_value) = schema.components.schemas.get(type_name) {
                    process_type(type_name, schema_value, generator)?;
                    return Ok((type_name.to_pascal_case(), false));
                }
            }
        }
    }

    // Default: return the last part of the reference as the type name
    let last_segment = ref_str.split('/').last().unwrap_or(ref_str);
    Ok((last_segment.to_pascal_case(), false))
}

fn generate_struct(
    name: &str,
    properties: &Map<String, Value>,
    required: Option<&Value>,
    generator: &mut TypeGenerator,
) -> Result<(), Box<dyn Error>> {
    let mut struct_def = String::new();
    struct_def.push_str(&format!(
        "#[derive(Debug, Clone, Serialize, Deserialize)]\npub struct {} {{\n",
        name.to_pascal_case()
    ));

    let required_fields = if let Some(Value::Array(req)) = required {
        req.iter()
            .filter_map(|v| if let Value::String(field) = v { Some(field.as_str()) } else { None })
            .collect::<HashSet<&str>>()
    } else {
        HashSet::new()
    };

    for (field_name, field_schema) in properties {
        let sanitized_field = sanitize_field_name(field_name);
        let (field_type, is_optional) = resolve_type(field_schema, generator)?;

        let is_required = required_fields.contains(field_name.as_str());

        if !is_required || is_optional {
            struct_def
                .push_str(&format!("    #[serde(skip_serializing_if = \"Option::is_none\")]\n"));
            struct_def.push_str(&format!("    pub {}: Option<{}>,\n", sanitized_field, field_type));
        } else {
            struct_def.push_str(&format!("    pub {}: {},\n", sanitized_field, field_type));
        }
    }

    struct_def.push_str("}\n\n");
    generator.types.insert(name.to_string(), struct_def);

    Ok(())
}

fn generate_enum(
    name: &str,
    variants: &[Value],
    generator: &mut TypeGenerator,
) -> Result<(), Box<dyn Error>> {
    let mut enum_def = String::new();
    enum_def.push_str(&format!(
        "#[derive(Debug, Clone, Serialize, Deserialize)]\npub enum {} {{\n",
        name.to_pascal_case()
    ));

    for variant in variants {
        match variant {
            Value::String(v) => {
                let variant_name = v.replace(|c: char| !c.is_alphanumeric(), "_").to_pascal_case();
                enum_def.push_str(&format!(
                    "    #[serde(rename = \"{}\")]\n    {},\n",
                    v, variant_name
                ));
            }
            Value::Number(n) => {
                let variant_name = format!("Num{}", n);
                enum_def
                    .push_str(&format!("    #[serde(rename = {})]\n    {},\n", n, variant_name));
            }
            Value::Bool(b) => {
                let variant_name = if *b { "True" } else { "False" };
                enum_def
                    .push_str(&format!("    #[serde(rename = {})]\n    {},\n", b, variant_name));
            }
            _ => {
                enum_def.push_str("    Unknown,\n");
            }
        }
    }

    enum_def.push_str("}\n\n");
    generator.enums.insert(name.to_string(), enum_def);

    Ok(())
}

fn generate_oneOf_enum(
    name: &str,
    variants: &[Value],
    generator: &mut TypeGenerator,
) -> Result<(), Box<dyn Error>> {
    // For oneOf/anyOf, we'll create an enum with different variants
    let mut enum_def = String::new();
    enum_def.push_str(&format!(
        "#[derive(Debug, Clone, Serialize, Deserialize)]\n#[serde(untagged)]\npub enum {} {{\n",
        name.to_pascal_case()
    ));

    for (i, variant) in variants.iter().enumerate() {
        let variant_name = if let Some(title) = variant.get("title").and_then(|t| t.as_str()) {
            title.to_pascal_case()
        } else {
            format!("Variant{}", i)
        };

        let (variant_type, _) = resolve_type(variant, generator)?;
        enum_def.push_str(&format!("    {}({}),\n", variant_name, variant_type));
    }

    enum_def.push_str("}\n\n");
    generator.enums.insert(name.to_string(), enum_def);

    Ok(())
}

fn generate_allOf_struct(
    name: &str,
    all_of: &[Value],
    generator: &mut TypeGenerator,
) -> Result<(), Box<dyn Error>> {
    let mut combined_properties = Map::new();
    let mut combined_required = Vec::new();

    for schema in all_of {
        match schema {
            Value::Object(obj) => {
                // If it's a reference, resolve it and merge its properties
                if let Some(Value::String(ref_str)) = obj.get("$ref") {
                    // Track reference for dependency
                    resolve_reference(ref_str, generator)?;
                }

                // Merge properties directly from this schema
                if let Some(Value::Object(props)) = obj.get("properties") {
                    for (k, v) in props {
                        combined_properties.insert(k.clone(), v.clone());
                    }
                }

                // Merge required fields
                if let Some(Value::Array(req)) = obj.get("required") {
                    for r in req {
                        combined_required.push(r.clone());
                    }
                }
            }
            _ => {}
        }
    }

    // Now generate a struct from the combined properties
    let required_value =
        if !combined_required.is_empty() { Some(Value::Array(combined_required)) } else { None };

    generate_struct(name, &combined_properties, required_value.as_ref(), generator)?;

    Ok(())
}

fn resolve_type(
    schema: &Value,
    generator: &mut TypeGenerator,
) -> Result<(String, bool), Box<dyn Error>> {
    match schema {
        Value::Object(obj) => {
            // Handle references
            if let Some(Value::String(ref_str)) = obj.get("$ref") {
                let (type_name, optional) = resolve_reference(ref_str, generator)?;
                return Ok((type_name, optional));
            }

            // Handle arrays
            if let Some(Value::String(type_str)) = obj.get("type") {
                if type_str == "array" {
                    if let Some(items) = obj.get("items") {
                        let (item_type, _) = resolve_type(items, generator)?;
                        return Ok((format!("Vec<{}>", item_type), false));
                    } else {
                        return Ok(("Vec<serde_json::Value>".to_string(), false));
                    }
                } else {
                    return Ok((map_json_type_to_rust(type_str, obj, generator)?, false));
                }
            }

            // Handle oneOf/anyOf
            for key in ["oneOf", "anyOf"] {
                if let Some(Value::Array(variants)) = obj.get(key) {
                    if !variants.is_empty() {
                        // Generate an anonymous enum or use the title if available
                        let enum_name = if let Some(Value::String(title)) = obj.get("title") {
                            title.to_pascal_case()
                        } else {
                            let mut hasher = std::collections::hash_map::DefaultHasher::new();
                            std::hash::Hash::hash(obj, &mut hasher);
                            format!("AnonymousEnum{}", std::hash::Hasher::finish(&hasher))
                        };

                        generate_oneOf_enum(&enum_name, variants, generator)?;
                        return Ok((enum_name.to_pascal_case(), false));
                    }
                }
            }

            // Handle allOf
            if let Some(Value::Array(all_of)) = obj.get("allOf") {
                if !all_of.is_empty() {
                    // Generate an anonymous struct or use the title if available
                    let struct_name = if let Some(Value::String(title)) = obj.get("title") {
                        title.to_pascal_case()
                    } else {
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        std::hash::Hash::hash(obj, &mut hasher);
                        format!("AnonymousStruct{}", std::hash::Hasher::finish(&hasher))
                    };

                    generate_allOf_struct(&struct_name, all_of, generator)?;
                    return Ok((struct_name.to_pascal_case(), false));
                }
            }

            // Handle objects with properties
            if let Some(Value::Object(properties)) = obj.get("properties") {
                let struct_name = if let Some(Value::String(title)) = obj.get("title") {
                    title.to_pascal_case()
                } else {
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    std::hash::Hash::hash(obj, &mut hasher);
                    format!("AnonymousStruct{}", std::hash::Hasher::finish(&hasher))
                };

                generate_struct(&struct_name, properties, obj.get("required"), generator)?;
                return Ok((struct_name, false));
            }
        }
        Value::String(type_str) => {
            return Ok((map_simple_type_to_rust(type_str), false));
        }
        _ => {}
    }

    // Default to Value for unknown schemas
    Ok(("serde_json::Value".to_string(), false))
}

fn map_json_type_to_rust(
    type_str: &str,
    schema: &Map<String, Value>,
    generator: &mut TypeGenerator,
) -> Result<String, Box<dyn Error>> {
    match type_str {
        "string" => {
            // Check for string format
            if let Some(Value::String(format)) = schema.get("format") {
                match format.as_str() {
                    "uri" => Ok("String".to_string()),
                    "date-time" => {
                        generator.imported_types.insert("chrono".to_string());
                        Ok("chrono::DateTime<chrono::Utc>".to_string())
                    }
                    _ => Ok("String".to_string()),
                }
            } else {
                Ok("String".to_string())
            }
        }
        "integer" => Ok("i64".to_string()),
        "number" => Ok("f64".to_string()),
        "boolean" => Ok("bool".to_string()),
        "null" => Ok("()".to_string()),
        "object" => {
            if let Some(Value::Object(add_props)) = schema.get("additionalProperties") {
                // Map-like object
                if let Some(Value::Bool(allow)) = add_props.get("additionalProperties") {
                    if *allow {
                        generator.imported_types.insert("std::collections::HashMap".to_string());
                        return Ok("HashMap<String, serde_json::Value>".to_string());
                    }
                }

                let (value_type, _) = resolve_type(&Value::Object(add_props.clone()), generator)?;
                generator.imported_types.insert("std::collections::HashMap".to_string());
                Ok(format!("HashMap<String, {}>", value_type))
            } else {
                // Generic object
                generator.imported_types.insert("std::collections::HashMap".to_string());
                Ok("HashMap<String, serde_json::Value>".to_string())
            }
        }
        "array" => {
            if let Some(items) = schema.get("items") {
                let (item_type, _) = resolve_type(items, generator)?;
                Ok(format!("Vec<{}>", item_type))
            } else {
                Ok("Vec<serde_json::Value>".to_string())
            }
        }
        _ => Ok("serde_json::Value".to_string()),
    }
}

fn map_simple_type_to_rust(type_str: &str) -> String {
    match type_str {
        "string" => "String".to_string(),
        "integer" => "i64".to_string(),
        "number" => "f64".to_string(),
        "boolean" => "bool".to_string(),
        "null" => "()".to_string(),
        _ => "serde_json::Value".to_string(),
    }
}

fn sanitize_field_name(name: &str) -> String {
    let snake_case = name.to_snake_case();
    let rust_keywords = [
        "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn",
        "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
        "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
        "use", "where", "while", "async", "await", "dyn", "abstract", "become", "box", "do",
        "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try",
    ];

    if rust_keywords.contains(&snake_case.as_str()) {
        format!("{}_field", snake_case)
    } else {
        snake_case
    }
}

fn generate_rust_code(generator: &TypeGenerator) -> Result<(), Box<dyn Error>> {
    let mut output = String::new();

    // Add header
    output.push_str("// Generated code from OpenRPC schema\n");
    output.push_str("// This file is auto-generated. DO NOT EDIT.\n\n");

    // Add imports
    output.push_str("use serde::{Deserialize, Serialize};\n");

    for import in &generator.imported_types {
        output.push_str(&format!("use {};\n", import));
    }

    output.push_str("\n");

    // Add enums (should come before types as they might be referenced)
    for (_, enum_def) in &generator.enums {
        output.push_str(enum_def);
    }

    // Add types
    for (_, type_def) in &generator.types {
        output.push_str(type_def);
    }

    // Write to file
    fs::write("starknet_types.rs", output)?;
    println!("Generated Rust types in starknet_types.rs");

    Ok(())
}
