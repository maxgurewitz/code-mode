use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

#[derive(Clone, Debug)]
pub(crate) struct Options {
    pub additional_properties: bool,
    pub ignore_min_and_max_items: bool,
    pub max_items: Option<usize>,
    pub unknown_any: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            additional_properties: true,
            ignore_min_and_max_items: false,
            max_items: Some(20),
            unknown_any: true,
        }
    }
}

pub(crate) fn schema_accepts_no_args(schema: &Value) -> bool {
    matches!(schema.get("properties"), Some(Value::Object(properties)) if properties.is_empty())
}

pub(crate) fn compile_named_declaration(
    schema: &Value,
    name: &str,
    options: &Options,
) -> Result<String> {
    let mut normalized = schema.clone();
    let root_name = to_safe_type_name(name);
    normalize_schema(&mut normalized, true, Some(&root_name), options)?;
    let compiler = Compiler::new(&normalized, options, &root_name)?;
    compiler.compile(&normalized, &root_name)
}

struct Compiler<'a> {
    options: &'a Options,
    definition_names: BTreeMap<String, String>,
    ordered_definition_refs: Vec<String>,
    root: &'a Value,
}

impl<'a> Compiler<'a> {
    fn new(root: &'a Value, options: &'a Options, root_name: &str) -> Result<Self> {
        let mut used_names = BTreeSet::new();
        used_names.insert(root_name.to_string());

        let mut definition_names = BTreeMap::new();
        let mut ordered_definition_refs = Vec::new();
        collect_definition_names(
            root,
            "#",
            &mut used_names,
            &mut definition_names,
            &mut ordered_definition_refs,
        );

        Ok(Self {
            options,
            definition_names,
            ordered_definition_refs,
            root,
        })
    }

    fn compile(&self, schema: &Value, root_name: &str) -> Result<String> {
        let mut declarations = Vec::new();

        for reference in &self.ordered_definition_refs {
            let schema = self
                .resolve_local_ref(reference)
                .with_context(|| format!("failed to resolve definition ref: {reference}"))?;
            let name = self
                .definition_names
                .get(reference)
                .expect("definition ref should have a generated name");
            declarations.push(self.render_declaration(schema, name)?);
        }

        declarations.push(self.render_declaration(schema, root_name)?);
        Ok(declarations.join("\n\n"))
    }

    fn render_declaration(&self, schema: &Value, name: &str) -> Result<String> {
        let comment = render_comment(schema, 0);
        let declaration = if should_use_interface(schema) {
            format!(
                "export interface {} {}",
                name,
                self.render_object_expression(schema, 0)?
            )
        } else {
            format_with_expression(
                &format!("export type {} = ", name),
                &self.render_type_expression(schema, 0)?,
                ";",
            )
        };

        Ok(match comment {
            Some(comment) => format!("{comment}\n{declaration}"),
            None => declaration,
        })
    }

    fn render_type_expression(&self, schema: &Value, indent: usize) -> Result<String> {
        match schema {
            Value::Bool(true) => Ok(self.default_unknown_type().to_string()),
            Value::Bool(false) => Ok("never".into()),
            Value::Object(object) => {
                if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                    return self.render_reference(reference, indent);
                }

                let mut intersection_parts = Vec::new();

                if let Some(Value::Array(all_of)) = object.get("allOf") {
                    for sub_schema in all_of {
                        intersection_parts.push(self.render_type_expression(sub_schema, indent)?);
                    }
                }

                if let Some(Value::Array(any_of)) = object.get("anyOf") {
                    let rendered = any_of
                        .iter()
                        .map(|sub_schema| self.render_type_expression(sub_schema, indent))
                        .collect::<Result<Vec<_>>>()?;
                    if !rendered.is_empty() {
                        intersection_parts.push(join_set_operation(rendered, '|'));
                    }
                }

                if let Some(Value::Array(one_of)) = object.get("oneOf") {
                    let rendered = one_of
                        .iter()
                        .map(|sub_schema| self.render_type_expression(sub_schema, indent))
                        .collect::<Result<Vec<_>>>()?;
                    if !rendered.is_empty() {
                        intersection_parts.push(join_set_operation(rendered, '|'));
                    }
                }

                if !intersection_parts.is_empty() {
                    let mut remainder = object.clone();
                    remainder.remove("allOf");
                    remainder.remove("anyOf");
                    remainder.remove("oneOf");

                    if schema_has_renderable_base(&remainder) {
                        intersection_parts.push(
                            self.render_non_composite_type(&Value::Object(remainder), indent)?,
                        );
                    }

                    return Ok(join_set_operation(intersection_parts, '&'));
                }

                self.render_non_composite_type(schema, indent)
            }
            _ => Ok(self.default_unknown_type().to_string()),
        }
    }

    fn render_non_composite_type(&self, schema: &Value, indent: usize) -> Result<String> {
        let Value::Object(object) = schema else {
            return Ok(self.default_unknown_type().to_string());
        };

        if let Some(enum_values) = object.get("enum").and_then(Value::as_array) {
            let variants = enum_values
                .iter()
                .map(json_value_to_ts_literal)
                .collect::<Option<Vec<_>>>()
                .unwrap_or_default();

            if !variants.is_empty() {
                return Ok(join_set_operation(variants, '|'));
            }

            return Ok(self.default_unknown_type().to_string());
        }

        if let Some(Value::Array(type_variants)) = object.get("type") {
            let variants = type_variants
                .iter()
                .filter_map(Value::as_str)
                .map(|schema_type| self.render_type_for_schema_type(object, schema_type, indent))
                .collect::<Result<Vec<_>>>()?;

            if !variants.is_empty() {
                return Ok(join_set_operation(variants, '|'));
            }
        }

        if is_object_like(object) {
            return self.render_object_expression(schema, indent);
        }

        if is_array_like(object) {
            return self.render_array_expression(schema, indent);
        }

        if let Some(schema_type) = object.get("type").and_then(Value::as_str) {
            return self.render_type_for_schema_type(object, schema_type, indent);
        }

        if object.is_empty() {
            return Ok(self.default_unknown_type().to_string());
        }

        Ok(self.default_unknown_type().to_string())
    }

    fn render_type_for_schema_type(
        &self,
        schema: &Map<String, Value>,
        schema_type: &str,
        indent: usize,
    ) -> Result<String> {
        match schema_type {
            "string" => Ok("string".into()),
            "number" | "integer" => Ok("number".into()),
            "boolean" => Ok("boolean".into()),
            "null" => Ok("null".into()),
            "object" => self.render_object_expression(&Value::Object(schema.clone()), indent),
            "array" => self.render_array_expression(&Value::Object(schema.clone()), indent),
            "any" => Ok(self.default_unknown_type().to_string()),
            _ => Ok(self.default_unknown_type().to_string()),
        }
    }

    fn render_reference(&self, reference: &str, indent: usize) -> Result<String> {
        if let Some(name) = self.definition_names.get(reference) {
            return Ok(name.clone());
        }

        let target = self
            .resolve_local_ref(reference)
            .with_context(|| format!("unsupported or unresolved $ref: {reference}"))?;
        self.render_type_expression(target, indent)
    }

    fn resolve_local_ref(&self, reference: &str) -> Option<&Value> {
        if reference == "#" {
            return Some(self.root);
        }

        let pointer = reference.strip_prefix('#')?;
        self.root.pointer(pointer)
    }

    fn render_object_expression(&self, schema: &Value, indent: usize) -> Result<String> {
        let object = schema
            .as_object()
            .context("expected object schema while rendering interface")?;

        let required = object
            .get("required")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();

        let mut members = Vec::new();

        if let Some(properties) = object.get("properties").and_then(Value::as_object) {
            let mut property_names = properties.keys().cloned().collect::<Vec<_>>();
            property_names.sort();

            for property_name in property_names {
                let property_schema = properties
                    .get(&property_name)
                    .expect("property name should resolve");

                if let Some(comment) = render_comment(property_schema, indent + 1) {
                    members.push(comment);
                }

                let property_type = self.render_type_expression(property_schema, indent + 1)?;
                let property_prefix = format!(
                    "{}{}{}: ",
                    indent_string(indent + 1),
                    escape_property_name(&property_name),
                    if required.contains(property_name.as_str()) {
                        ""
                    } else {
                        "?"
                    }
                );
                members.push(format_with_expression(
                    &property_prefix,
                    &property_type,
                    ";",
                ));
            }
        }

        if let Some(index_signature) = self.render_index_signature(object, indent + 1)? {
            members.push(index_signature);
        }

        if members.is_empty() {
            return Ok(format!("{{\n{}}}", indent_string(indent)));
        }

        Ok(format!(
            "{{\n{}\n{}}}",
            members.join("\n"),
            indent_string(indent)
        ))
    }

    fn render_index_signature(
        &self,
        object: &Map<String, Value>,
        indent: usize,
    ) -> Result<Option<String>> {
        let mut index_types = Vec::new();

        if let Some(pattern_properties) = object.get("patternProperties").and_then(Value::as_object)
        {
            let mut pattern_keys = pattern_properties.keys().cloned().collect::<Vec<_>>();
            pattern_keys.sort();
            for key in pattern_keys {
                let pattern_schema = pattern_properties
                    .get(&key)
                    .expect("pattern property should resolve");
                index_types.push(self.render_type_expression(pattern_schema, indent)?);
            }
        }

        match object.get("additionalProperties") {
            Some(Value::Bool(true)) => index_types.push(self.default_unknown_type().to_string()),
            Some(Value::Object(schema)) => index_types
                .push(self.render_type_expression(&Value::Object(schema.clone()), indent)?),
            _ => {}
        }

        let index_types = dedupe_preserving_order(index_types);
        if index_types.is_empty() {
            return Ok(None);
        }

        let index_type = join_set_operation(index_types, '|');
        Ok(Some(format!(
            "{}[k: string]: {};",
            indent_string(indent),
            index_type
        )))
    }

    fn render_array_expression(&self, schema: &Value, _indent: usize) -> Result<String> {
        let object = schema
            .as_object()
            .context("expected object schema while rendering array")?;

        let min_items = object
            .get("minItems")
            .and_then(as_usize)
            .unwrap_or_default();
        let max_items = object.get("maxItems").and_then(as_usize);

        match object.get("items") {
            Some(Value::Array(tuple_items)) => {
                let tuple_types = tuple_items
                    .iter()
                    .map(|item| self.render_type_expression(item, 0))
                    .collect::<Result<Vec<_>>>()?;
                let spread = match object.get("additionalItems") {
                    Some(Value::Bool(true)) => Some(self.default_unknown_type().to_string()),
                    Some(Value::Object(schema)) => {
                        Some(self.render_type_expression(&Value::Object(schema.clone()), 0)?)
                    }
                    _ => None,
                };
                Ok(self.render_tuple(tuple_types, spread, min_items, max_items))
            }
            Some(item_schema) => {
                let item_type = self.render_type_expression(item_schema, 0)?;
                Ok(format!("{}[]", wrap_array_item_type(&item_type)))
            }
            None => {
                if min_items > 0 || max_items.is_some() {
                    let tuple_len = max_items.unwrap_or(min_items);
                    let tuple_types = vec![self.default_unknown_type().to_string(); tuple_len];
                    let spread = if max_items.is_some() {
                        None
                    } else {
                        Some(self.default_unknown_type().to_string())
                    };
                    Ok(self.render_tuple(tuple_types, spread, min_items, max_items))
                } else {
                    Ok(format!("{}[]", self.default_unknown_type()))
                }
            }
        }
    }

    fn render_tuple(
        &self,
        tuple_types: Vec<String>,
        spread: Option<String>,
        min_items: usize,
        max_items: Option<usize>,
    ) -> String {
        let mut tuple_types = tuple_types;
        let mut spread = spread;

        if min_items > 0 && min_items > tuple_types.len() && spread.is_none() && max_items.is_none()
        {
            spread = Some(self.default_unknown_type().to_string());
        }

        if let Some(max_items) = max_items {
            if max_items > tuple_types.len() && spread.is_none() {
                while tuple_types.len() < max_items {
                    tuple_types.push(self.default_unknown_type().to_string());
                }
            }
        }

        if tuple_types.len() > min_items {
            let mut current = tuple_types[..min_items].to_vec();
            let mut variants = Vec::new();

            if !current.is_empty() || min_items == 0 {
                variants.push(format_tuple(&current, None));
            }

            for (idx, item_type) in tuple_types[min_items..].iter().enumerate() {
                current.push(item_type.clone());
                let is_last = idx + min_items + 1 == tuple_types.len();
                let tuple_variant = if is_last {
                    format_tuple(&current, spread.as_deref())
                } else {
                    format_tuple(&current, None)
                };
                variants.push(tuple_variant);
            }

            return join_set_operation(variants, '|');
        }

        format_tuple(&tuple_types, spread.as_deref())
    }

    fn default_unknown_type(&self) -> &'static str {
        if self.options.unknown_any {
            "unknown"
        } else {
            "any"
        }
    }
}

fn collect_definition_names(
    schema: &Value,
    path: &str,
    used_names: &mut BTreeSet<String>,
    definition_names: &mut BTreeMap<String, String>,
    ordered_definition_refs: &mut Vec<String>,
) {
    let Some(object) = schema.as_object() else {
        return;
    };

    if let Some(definitions) = object.get("$defs").and_then(Value::as_object) {
        let mut definition_keys = definitions.keys().cloned().collect::<Vec<_>>();
        definition_keys.sort();

        for definition_key in definition_keys {
            let definition_schema = definitions
                .get(&definition_key)
                .expect("definition key should resolve");
            let reference = format!(
                "{path}/$defs/{}",
                escape_json_pointer_token(&definition_key)
            );
            if !definition_names.contains_key(&reference) {
                let base_name = definition_schema
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or(&definition_key);
                let name = make_unique_name(base_name, used_names);
                definition_names.insert(reference.clone(), name);
                ordered_definition_refs.push(reference.clone());
            }
            collect_definition_names(
                definition_schema,
                &reference,
                used_names,
                definition_names,
                ordered_definition_refs,
            );
        }
    }

    recurse_child_schemas(object, path, |child, child_path| {
        collect_definition_names(
            child,
            child_path,
            used_names,
            definition_names,
            ordered_definition_refs,
        )
    });
}

fn normalize_schema(
    schema: &mut Value,
    is_root: bool,
    root_name: Option<&str>,
    options: &Options,
) -> Result<()> {
    let Some(object) = schema.as_object_mut() else {
        return Ok(());
    };

    if let Some(id) = object.get("id").cloned() {
        match object.get("$id") {
            Some(existing) if existing != &id => {
                bail!("schema must define either id or $id, not both with different values")
            }
            Some(_) => {
                object.remove("id");
            }
            None => {
                object.insert("$id".into(), id);
                object.remove("id");
            }
        }
    }

    if let Some(definitions) = object.get("definitions").cloned() {
        match object.get("$defs") {
            Some(existing) if existing != &definitions => {
                bail!("schema must define either definitions or $defs, not both")
            }
            Some(_) => {
                object.remove("definitions");
            }
            None => {
                object.insert("$defs".into(), definitions);
                object.remove("definitions");
            }
        }
    }

    if let Some(Value::Array(types)) = object.get("type") {
        if types.len() == 1 {
            object.insert("type".into(), types[0].clone());
        }
    }

    if object
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|values| values.iter().any(Value::is_null))
    {
        if let Some(Value::Array(types)) = object.get_mut("type") {
            types.retain(|value| value.as_str() != Some("null"));
            if types.len() == 1 {
                let remaining = types[0].clone();
                object.insert("type".into(), remaining);
            }
        }
    }

    if let Some(constant) = object.get("const").cloned() {
        object.insert("enum".into(), Value::Array(vec![constant]));
        object.remove("const");
    }

    if matches!(object.get("required"), Some(Value::Bool(false))) {
        object.insert("required".into(), Value::Array(Vec::new()));
    }

    if is_object_like(object) && !object.contains_key("required") {
        object.insert("required".into(), Value::Array(Vec::new()));
    }

    if is_object_like(object)
        && !object.contains_key("additionalProperties")
        && !object.contains_key("patternProperties")
    {
        object.insert(
            "additionalProperties".into(),
            Value::Bool(options.additional_properties),
        );
    }

    if let Some(Value::String(description)) = object.get_mut("description") {
        *description = description.replace("*/", "*\\/");
    }

    if is_root && !object.contains_key("$id") {
        object.insert(
            "$id".into(),
            Value::String(to_safe_type_name(root_name.unwrap_or("Schema"))),
        );
    }

    recurse_child_schemas_mut(object, |child, child_is_root| {
        normalize_schema(child, child_is_root, None, options)
    })?;

    if options.ignore_min_and_max_items {
        object.remove("minItems");
        object.remove("maxItems");
        return Ok(());
    }

    if is_array_like(object) && !object.contains_key("minItems") {
        object.insert("minItems".into(), Value::from(0));
    }

    let min_items = object
        .get("minItems")
        .and_then(as_usize)
        .unwrap_or_default();
    if let (Some(max_items), Some(limit)) =
        (object.get("maxItems").and_then(as_usize), options.max_items)
    {
        if max_items.saturating_sub(min_items) > limit {
            object.remove("maxItems");
        }
    }

    let max_items = object.get("maxItems").and_then(as_usize);
    let has_max = max_items.is_some();
    let has_min = min_items > 0;

    if let Some(items) = object.get("items").cloned() {
        match items {
            Value::Object(_) if has_max || has_min => {
                let tuple_len = max_items.unwrap_or(min_items);
                let tuple_items = vec![items.clone(); tuple_len];
                if !has_max {
                    object.insert("additionalItems".into(), items);
                }
                object.insert("items".into(), Value::Array(tuple_items));
            }
            Value::Array(mut tuple_items) => {
                if let Some(max_items) = max_items {
                    if max_items < tuple_items.len() {
                        tuple_items.truncate(max_items);
                        object.insert("items".into(), Value::Array(tuple_items));
                    }
                }
            }
            _ => {}
        }
    }

    Ok(())
}

fn recurse_child_schemas<F>(object: &Map<String, Value>, path: &str, mut callback: F)
where
    F: FnMut(&Value, &str),
{
    for key in ["properties", "patternProperties", "$defs"] {
        if let Some(children) = object.get(key).and_then(Value::as_object) {
            let mut child_keys = children.keys().cloned().collect::<Vec<_>>();
            child_keys.sort();
            for child_key in child_keys {
                let child = children.get(&child_key).expect("child key should resolve");
                let child_path = format!("{path}/{key}/{}", escape_json_pointer_token(&child_key));
                callback(child, &child_path);
            }
        }
    }

    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(children) = object.get(key).and_then(Value::as_array) {
            for (idx, child) in children.iter().enumerate() {
                let child_path = format!("{path}/{key}/{idx}");
                callback(child, &child_path);
            }
        }
    }

    for key in ["additionalProperties", "additionalItems", "not", "items"] {
        match object.get(key) {
            Some(Value::Object(child)) => {
                callback(&Value::Object(child.clone()), &format!("{path}/{key}"))
            }
            Some(Value::Array(children)) if key == "items" => {
                for (idx, child) in children.iter().enumerate() {
                    callback(child, &format!("{path}/{key}/{idx}"));
                }
            }
            _ => {}
        }
    }
}

fn recurse_child_schemas_mut<F>(object: &mut Map<String, Value>, mut callback: F) -> Result<()>
where
    F: FnMut(&mut Value, bool) -> Result<()>,
{
    for key in ["properties", "patternProperties", "$defs"] {
        if let Some(children) = object.get_mut(key).and_then(Value::as_object_mut) {
            let mut child_keys = children.keys().cloned().collect::<Vec<_>>();
            child_keys.sort();
            for child_key in child_keys {
                let child = children
                    .get_mut(&child_key)
                    .expect("child key should resolve");
                callback(child, false)?;
            }
        }
    }

    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(children) = object.get_mut(key).and_then(Value::as_array_mut) {
            for child in children {
                callback(child, false)?;
            }
        }
    }

    for key in ["additionalProperties", "additionalItems", "not"] {
        if let Some(child) = object.get_mut(key) {
            callback(child, false)?;
        }
    }

    if let Some(items) = object.get_mut("items") {
        match items {
            Value::Array(children) => {
                for child in children {
                    callback(child, false)?;
                }
            }
            child => callback(child, false)?,
        }
    }

    Ok(())
}

fn should_use_interface(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };

    if object.contains_key("$ref")
        || object.contains_key("enum")
        || object.contains_key("allOf")
        || object.contains_key("anyOf")
        || object.contains_key("oneOf")
    {
        return false;
    }

    !matches!(object.get("type"), Some(Value::Array(_))) && is_object_like(object)
}

fn schema_has_renderable_base(object: &Map<String, Value>) -> bool {
    object.contains_key("$ref")
        || object.contains_key("enum")
        || object.contains_key("type")
        || object.contains_key("properties")
        || object.contains_key("patternProperties")
        || object.contains_key("additionalProperties")
        || object.contains_key("items")
        || object.contains_key("additionalItems")
        || object.contains_key("minItems")
        || object.contains_key("maxItems")
        || object.contains_key("required")
}

fn is_object_like(object: &Map<String, Value>) -> bool {
    object.contains_key("properties")
        || object.contains_key("patternProperties")
        || object.contains_key("additionalProperties")
        || object.get("type").and_then(Value::as_str) == Some("object")
}

fn is_array_like(object: &Map<String, Value>) -> bool {
    object.contains_key("items")
        || object.contains_key("minItems")
        || object.contains_key("maxItems")
        || object.get("type").and_then(Value::as_str) == Some("array")
}

fn render_comment(schema: &Value, indent: usize) -> Option<String> {
    let object = schema.as_object()?;
    let description = object.get("description").and_then(Value::as_str);
    let deprecated = object
        .get("deprecated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let min_items = object.get("minItems").and_then(as_usize);
    let max_items = object.get("maxItems").and_then(as_usize);

    let mut lines = Vec::new();
    if deprecated {
        lines.push("@deprecated".to_string());
    }

    if let Some(description) = description {
        lines.extend(description.lines().map(str::to_owned));
    }

    if is_array_like(object) {
        if let Some(min_items) = min_items.filter(|min_items| *min_items > 0) {
            lines.push(format!("@minItems {min_items}"));
        }
        if let Some(max_items) = max_items {
            lines.push(format!("@maxItems {max_items}"));
        }
    }

    if lines.is_empty() {
        return None;
    }

    let indent = indent_string(indent);
    let body = lines
        .into_iter()
        .map(|line| format!("{indent} * {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!("{indent}/**\n{body}\n{indent} */"))
}

fn json_value_to_ts_literal(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => serde_json::to_string(value).ok(),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Null => Some("null".into()),
        _ => None,
    }
}

fn make_unique_name(input: &str, used_names: &mut BTreeSet<String>) -> String {
    let base_name = to_safe_type_name(input);
    if !used_names.contains(&base_name) {
        used_names.insert(base_name.clone());
        return base_name;
    }

    let mut suffix = 1usize;
    loop {
        let candidate = format!("{base_name}{suffix}");
        if !used_names.contains(&candidate) {
            used_names.insert(candidate.clone());
            return candidate;
        }
        suffix += 1;
    }
}

fn to_safe_type_name(input: &str) -> String {
    let mut output = String::new();
    let mut capitalize_next = true;

    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' {
            if output.is_empty() && ch.is_ascii_digit() {
                output.push('N');
            }

            if ch == '_' {
                capitalize_next = true;
                continue;
            }

            if capitalize_next {
                output.push(ch.to_ascii_uppercase());
                capitalize_next = false;
            } else {
                output.push(ch);
            }
        } else {
            capitalize_next = true;
        }
    }

    if output.is_empty() {
        "NoName".into()
    } else {
        output
    }
}

fn escape_property_name(property_name: &str) -> String {
    let mut chars = property_name.chars();
    match chars.next() {
        Some(first) if (first == '_' || first == '$' || first.is_ascii_alphabetic()) => {
            if chars.all(|ch| ch == '_' || ch == '$' || ch.is_ascii_alphanumeric()) {
                property_name.to_string()
            } else {
                serde_json::to_string(property_name)
                    .unwrap_or_else(|_| format!("\"{property_name}\""))
            }
        }
        _ => {
            serde_json::to_string(property_name).unwrap_or_else(|_| format!("\"{property_name}\""))
        }
    }
}

fn wrap_array_item_type(item_type: &str) -> String {
    if item_type.contains(" | ")
        || item_type.contains(" & ")
        || item_type.starts_with('{')
        || item_type.starts_with('[')
        || item_type.starts_with('"')
    {
        format!("({item_type})")
    } else {
        item_type.to_string()
    }
}

fn format_tuple(items: &[String], spread: Option<&str>) -> String {
    let mut parts = items.to_vec();
    if let Some(spread) = spread {
        parts.push(format!("...{}[]", wrap_array_item_type(spread)));
    }
    format!("[{}]", parts.join(", "))
}

fn join_set_operation(items: Vec<String>, operator: char) -> String {
    let items = dedupe_preserving_order(items);
    if items.len() <= 1 {
        return items.into_iter().next().unwrap_or_else(|| "unknown".into());
    }

    format!("({})", items.join(&format!(" {operator} ")))
}

fn dedupe_preserving_order(items: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for item in items {
        if seen.insert(item.clone()) {
            deduped.push(item);
        }
    }
    deduped
}

fn format_with_expression(prefix: &str, expression: &str, suffix: &str) -> String {
    let mut lines = expression.lines();
    let first_line = lines.next().unwrap_or_default();
    let mut output = String::new();
    output.push_str(prefix);
    output.push_str(first_line);

    for line in lines {
        output.push('\n');
        output.push_str(line);
    }

    output.push_str(suffix);
    output
}

fn indent_string(indent: usize) -> String {
    "  ".repeat(indent)
}

fn as_usize(value: &Value) -> Option<usize> {
    value.as_u64().and_then(|value| usize::try_from(value).ok())
}

fn escape_json_pointer_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
fn normalize_for_test(schema: &Value, options: &Options) -> Result<Value> {
    let mut normalized = schema.clone();
    normalize_schema(&mut normalized, true, Some("Fixture"), options)?;
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_options() -> Options {
        Options::default()
    }

    #[test]
    fn normalizes_id_to_dollar_id_from_upstream_fixture() -> Result<()> {
        let input = serde_json::json!({
            "properties": {
                "b": {
                    "id": "b",
                    "type": "object",
                    "additionalProperties": false,
                    "required": []
                }
            },
            "additionalProperties": false,
            "required": [],
            "id": "a"
        });

        let normalized = normalize_for_test(&input, &default_options())?;
        assert_eq!(
            normalized,
            serde_json::json!({
                "properties": {
                    "b": {
                        "$id": "b",
                        "type": "object",
                        "additionalProperties": false,
                        "required": []
                    }
                },
                "additionalProperties": false,
                "required": [],
                "$id": "a"
            })
        );

        Ok(())
    }

    #[test]
    fn defaults_additional_properties_from_upstream_fixture() -> Result<()> {
        let input = serde_json::json!({
            "$id": "foo",
            "type": ["object"],
            "properties": {
                "a": {
                    "type": "integer",
                    "$id": "a"
                }
            },
            "required": []
        });

        let normalized = normalize_for_test(&input, &default_options())?;
        assert_eq!(
            normalized,
            serde_json::json!({
                "$id": "foo",
                "type": "object",
                "properties": {
                    "a": {
                        "type": "integer",
                        "$id": "a"
                    }
                },
                "required": [],
                "additionalProperties": true
            })
        );

        Ok(())
    }

    #[test]
    fn normalizes_const_to_singleton_enum_from_upstream_fixture() -> Result<()> {
        let input = serde_json::json!({
            "$id": "foo",
            "const": "foobar"
        });

        let normalized = normalize_for_test(&input, &default_options())?;
        assert_eq!(
            normalized,
            serde_json::json!({
                "$id": "foo",
                "enum": ["foobar"]
            })
        );

        Ok(())
    }

    #[test]
    fn normalizes_schema_items_from_upstream_fixture() -> Result<()> {
        let input = serde_json::json!({
            "$id": "foo",
            "type": "object",
            "properties": {
                "typedMinBounded": {
                    "items": {
                        "type": "string"
                    },
                    "minItems": 2
                },
                "typedMaxBounded": {
                    "items": {
                        "type": "string"
                    },
                    "maxItems": 2
                }
            },
            "additionalProperties": false
        });

        let normalized = normalize_for_test(&input, &default_options())?;
        assert_eq!(
            normalized,
            serde_json::json!({
                "$id": "foo",
                "type": "object",
                "properties": {
                    "typedMinBounded": {
                        "items": [
                            { "type": "string" },
                            { "type": "string" }
                        ],
                        "additionalItems": { "type": "string" },
                        "minItems": 2
                    },
                    "typedMaxBounded": {
                        "items": [
                            { "type": "string" },
                            { "type": "string" }
                        ],
                        "maxItems": 2,
                        "minItems": 0
                    }
                },
                "additionalProperties": false,
                "required": []
            })
        );

        Ok(())
    }

    #[test]
    fn compiles_basics_schema_from_upstream_fixture() -> Result<()> {
        let schema = serde_json::json!({
            "title": "Example Schema",
            "type": "object",
            "properties": {
                "firstName": { "type": "string" },
                "lastName": { "id": "lastName", "type": "string" },
                "age": {
                    "description": "Age in years",
                    "type": "integer",
                    "minimum": 0
                },
                "height": { "$id": "height", "type": "number" },
                "favoriteFoods": { "type": "array" },
                "likesDogs": { "type": "boolean" }
            },
            "required": ["firstName", "lastName"]
        });

        let declaration = compile_named_declaration(&schema, "ExampleSchema", &default_options())?;
        assert_eq!(
            declaration,
            r#"export interface ExampleSchema {
  /**
   * Age in years
   */
  age?: number;
  favoriteFoods?: unknown[];
  firstName: string;
  height?: number;
  lastName: string;
  likesDogs?: boolean;
  [k: string]: unknown;
}"#
        );

        Ok(())
    }

    #[test]
    fn compiles_additional_properties_schema_from_upstream_fixture() -> Result<()> {
        let schema = serde_json::json!({
            "title": "AdditionalProperties",
            "type": "object",
            "properties": {
                "foo": { "type": "string" }
            },
            "additionalProperties": { "type": "number" }
        });

        let declaration =
            compile_named_declaration(&schema, "AdditionalProperties", &default_options())?;
        assert_eq!(
            declaration,
            r#"export interface AdditionalProperties {
  foo?: string;
  [k: string]: number;
}"#
        );

        Ok(())
    }

    #[test]
    fn compiles_array_max_min_items_schema_from_upstream_fixture() -> Result<()> {
        let schema = serde_json::json!({
            "title": "ArrayMaxMinItems",
            "type": "object",
            "properties": {
                "array": {
                    "type": "object",
                    "properties": {
                        "withMinItems": {
                            "type": "array",
                            "items": { "type": "string" },
                            "minItems": 3
                        },
                        "withMaxItems": {
                            "type": "array",
                            "items": { "type": "string" },
                            "maxItems": 3
                        },
                        "withTupleMaxItems": {
                            "type": "array",
                            "items": [{ "enum": [1] }, { "enum": [2] }, { "enum": [3] }],
                            "maxItems": 2
                        }
                    },
                    "additionalProperties": false
                }
            },
            "additionalProperties": false
        });

        let declaration =
            compile_named_declaration(&schema, "ArrayMaxMinItems", &default_options())?;
        assert_eq!(
            declaration,
            r#"export interface ArrayMaxMinItems {
  array?: {
    /**
     * @maxItems 3
     */
    withMaxItems?: ([] | [string] | [string, string] | [string, string, string]);
    /**
     * @minItems 3
     */
    withMinItems?: [string, string, string, ...string[]];
    /**
     * @maxItems 2
     */
    withTupleMaxItems?: ([] | [1] | [1, 2]);
  };
}"#
        );

        Ok(())
    }

    #[test]
    fn compiles_local_defs_refs_into_named_declarations() -> Result<()> {
        let schema = serde_json::json!({
            "type": "object",
            "$defs": {
                "address": {
                    "type": "object",
                    "properties": {
                        "city": { "type": "string" }
                    },
                    "required": ["city"],
                    "additionalProperties": false
                }
            },
            "properties": {
                "shipping": { "$ref": "#/$defs/address" }
            },
            "required": ["shipping"],
            "additionalProperties": false
        });

        let declaration = compile_named_declaration(&schema, "Order", &default_options())?;
        assert_eq!(
            declaration,
            r#"export interface Address {
  city: string;
}

export interface Order {
  shipping: Address;
}"#
        );

        Ok(())
    }
}
