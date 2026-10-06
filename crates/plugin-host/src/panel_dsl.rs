//! 插件面板的声明式 UI DSL（T6.3 方案 C，审批定案：HTML/iframe 延后到 1.0 后）。
//!
//! # 为什么在宿主侧校验而不是只靠前端兜底
//!
//! 面板内容是插件生成的任意 JSON，直接进 React 意味着把"格式错误"推迟到
//! 渲染时才暴露。宿主在 `render_panel` 返回前先校验：未知块类型、缺字段、
//! 超上限（块数/行数/字符串长度）都在引擎层变成结构化错误——前端拿到的
//! DSL 永远是良构的；前端仍然保留兜底错误卡片（纵深防御 + T6.3 验收）。
//!
//! # 表达能力的刻意取舍
//!
//! 只有七种块：heading / text / keyValue / table / list / progress / button。
//! 没有自由布局、没有样式覆盖、没有脚本——这是方案 C 的核心交易：
//! 用表达能力换"永远和主题一致 + 永远不会注入"。button 不直接执行动作，
//! 只引用插件已注册的命令 id（动作链路复用命令权限与审计）。

use crate::runtime::HostError;
use serde_json::Value;

/// 单个面板允许的最大块数。
pub const MAX_BLOCKS: usize = 200;
/// 表格最大行数。
pub const MAX_TABLE_ROWS: usize = 200;
/// 表格最大列数。
pub const MAX_TABLE_COLUMNS: usize = 12;
/// 列表最大条数。
pub const MAX_LIST_ITEMS: usize = 200;
/// 键值对最大条数。
pub const MAX_KEY_VALUE_ENTRIES: usize = 50;
/// 单个字符串字段的最大字节数。
pub const MAX_STRING_BYTES: usize = 8 * 1024;

/// 合法的块类型名（封闭集合）。
const BLOCK_TYPES: [&str; 7] = [
    "heading", "text", "keyValue", "table", "list", "progress", "button",
];

/// text 块允许的色调（映射到设计 token 的语义色）。
const TEXT_TONES: [&str; 5] = ["plain", "muted", "success", "warning", "danger"];

/// 校验面板 DSL（入口为 `fd_render_panel` 返回的 JSON 字节）。
///
/// 只校验不转换：通过后原样把 JSON 交给前端渲染，避免宿主重序列化造成
/// 字段顺序/空白抖动（diff 友好）。
pub fn validate_panel_dsl(bytes: &[u8]) -> Result<(), HostError> {
    let invalid = |reason: String| HostError::InvalidArgument("panel", reason);
    let root: Value = serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("not valid JSON: {error}")))?;
    let blocks = root
        .as_array()
        .ok_or_else(|| invalid("root must be an array of blocks".to_owned()))?;
    if blocks.len() > MAX_BLOCKS {
        return Err(invalid(format!(
            "too many blocks: {} > {MAX_BLOCKS}",
            blocks.len()
        )));
    }
    for (index, block) in blocks.iter().enumerate() {
        validate_block(block).map_err(|error| invalid(format!("block #{index}: {error}")))?;
    }
    Ok(())
}

fn validate_block(block: &Value) -> Result<(), HostError> {
    let obj = block
        .as_object()
        .ok_or_else(|| HostError::InvalidArgument("block", "must be an object".to_owned()))?;
    let block_type = obj
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| HostError::InvalidArgument("type", "must be a string".to_owned()))?;
    if !BLOCK_TYPES.contains(&block_type) {
        return Err(HostError::InvalidArgument(
            "type",
            format!("unknown block type `{block_type}`"),
        ));
    }
    /// 读取一个字符串字段并做长度上限校验（独立 fn 避免闭包生命周期推导问题）。
    /// key 是编译期字面量，满足 HostError::InvalidArgument 的 &'static str。
    fn text_field(
        obj: &serde_json::Map<String, Value>,
        key: &'static str,
    ) -> Result<String, HostError> {
        let value = obj
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| HostError::InvalidArgument(key, "must be a string".to_owned()))?;
        if value.len() > MAX_STRING_BYTES {
            return Err(HostError::InvalidArgument(
                key,
                format!("exceeds {MAX_STRING_BYTES} bytes"),
            ));
        }
        Ok(value.to_owned())
    }
    let text = |key: &'static str| text_field(obj, key);
    match block_type {
        "heading" => {
            text("text")?;
        }
        "text" => {
            text("text")?;
            if let Some(tone) = obj.get("tone") {
                let tone = tone.as_str().ok_or_else(|| {
                    HostError::InvalidArgument("tone", "must be a string".to_owned())
                })?;
                if !TEXT_TONES.contains(&tone) {
                    return Err(HostError::InvalidArgument(
                        "tone",
                        format!("unknown tone `{tone}`"),
                    ));
                }
            }
        }
        "keyValue" => {
            let entries = obj
                .get("entries")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    HostError::InvalidArgument("entries", "must be an array".to_owned())
                })?;
            if entries.len() > MAX_KEY_VALUE_ENTRIES {
                return Err(HostError::InvalidArgument(
                    "entries",
                    format!("more than {MAX_KEY_VALUE_ENTRIES} entries"),
                ));
            }
            for entry in entries {
                let pair = entry.as_array().ok_or_else(|| {
                    HostError::InvalidArgument("entries", "items must be [key, value]".to_owned())
                })?;
                if pair.len() != 2 || pair.iter().any(|part| !part.is_string()) {
                    return Err(HostError::InvalidArgument(
                        "entries",
                        "items must be [key, value] pairs of strings".to_owned(),
                    ));
                }
            }
        }
        "table" => {
            let columns = obj
                .get("columns")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    HostError::InvalidArgument("columns", "must be an array".to_owned())
                })?;
            if columns.is_empty() || columns.len() > MAX_TABLE_COLUMNS {
                return Err(HostError::InvalidArgument(
                    "columns",
                    format!("must contain 1..={MAX_TABLE_COLUMNS} columns"),
                ));
            }
            let rows = obj
                .get("rows")
                .and_then(Value::as_array)
                .ok_or_else(|| HostError::InvalidArgument("rows", "must be an array".to_owned()))?;
            if rows.len() > MAX_TABLE_ROWS {
                return Err(HostError::InvalidArgument(
                    "rows",
                    format!("more than {MAX_TABLE_ROWS} rows"),
                ));
            }
            for row in rows {
                let cells = row.as_array().ok_or_else(|| {
                    HostError::InvalidArgument("rows", "rows must be arrays".to_owned())
                })?;
                if cells.len() != columns.len() {
                    return Err(HostError::InvalidArgument(
                        "rows",
                        "every row must match the column count".to_owned(),
                    ));
                }
                for cell in cells {
                    if !cell.is_string() {
                        return Err(HostError::InvalidArgument(
                            "rows",
                            "cells must be strings".to_owned(),
                        ));
                    }
                }
            }
        }
        "list" => {
            let items = obj.get("items").and_then(Value::as_array).ok_or_else(|| {
                HostError::InvalidArgument("items", "must be an array".to_owned())
            })?;
            if items.len() > MAX_LIST_ITEMS {
                return Err(HostError::InvalidArgument(
                    "items",
                    format!("more than {MAX_LIST_ITEMS} items"),
                ));
            }
            if items.iter().any(|item| !item.is_string()) {
                return Err(HostError::InvalidArgument(
                    "items",
                    "items must be strings".to_owned(),
                ));
            }
        }
        "progress" => {
            text("label")?;
            let value = obj.get("value").and_then(Value::as_i64).ok_or_else(|| {
                HostError::InvalidArgument("value", "must be an integer".to_owned())
            })?;
            if !(0..=100).contains(&value) {
                return Err(HostError::InvalidArgument(
                    "value",
                    format!("must be in 0..=100, got {value}"),
                ));
            }
        }
        "button" => {
            text("label")?;
            text("command")?;
        }
        _ => unreachable!("block type was validated against the whitelist above"),
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;

    fn validate(value: Value) -> Result<(), HostError> {
        validate_panel_dsl(&serde_json::to_vec(&value).unwrap())
    }

    #[test]
    fn a_panel_with_all_seven_block_types_is_accepted() {
        let dsl = json!([
            { "type": "heading", "text": "Stats" },
            { "type": "text", "text": "last 30 days", "tone": "muted" },
            { "type": "keyValue", "entries": [["branch", "main"], ["ahead", "3"]] },
            { "type": "table", "columns": ["author", "commits"], "rows": [["a", "12"], ["b", "7"]] },
            { "type": "list", "items": ["large file: a.bin", "secret: .env"] },
            { "type": "progress", "label": "quota", "value": 42 },
            { "type": "button", "command": "com.example.x.refresh", "label": "Refresh" }
        ]);
        assert!(validate(dsl).is_ok());
    }

    #[test]
    fn the_dsl_root_must_be_an_array() {
        assert!(validate(json!({"type": "text"})).is_err());
        assert!(validate(json!("nope")).is_err());
    }

    #[test]
    fn unknown_block_types_are_rejected_with_the_index() {
        let error = validate(json!([{ "type": "iframe", "src": "https://evil" }])).unwrap_err();
        assert!(error.to_string().contains("block #0"), "{error}");
        assert!(error.to_string().contains("iframe"), "{error}");
    }

    #[test]
    fn missing_or_wrong_typed_fields_are_rejected() {
        assert!(validate(json!([{ "type": "heading" }])).is_err());
        assert!(validate(json!([{ "type": "text", "text": 42 }])).is_err());
        assert!(validate(json!([{ "type": "progress", "label": "x", "value": "80" }])).is_err());
        assert!(validate(json!([{ "type": "button", "label": "go" }])).is_err());
    }

    #[test]
    fn text_tones_are_whitelisted() {
        assert!(validate(json!([{ "type": "text", "text": "x", "tone": "danger" }])).is_ok());
        assert!(validate(json!([{ "type": "text", "text": "x", "tone": "neon" }])).is_err());
    }

    #[test]
    fn progress_values_are_bounded_to_0_100() {
        assert!(validate(json!([{ "type": "progress", "label": "x", "value": 0 }])).is_ok());
        assert!(validate(json!([{ "type": "progress", "label": "x", "value": 100 }])).is_ok());
        assert!(validate(json!([{ "type": "progress", "label": "x", "value": 101 }])).is_err());
        assert!(validate(json!([{ "type": "progress", "label": "x", "value": -1 }])).is_err());
    }

    #[test]
    fn table_rows_must_match_the_column_count() {
        let dsl = json!([{ "type": "table", "columns": ["a", "b"], "rows": [["1"]] }]);
        let error = validate(dsl).unwrap_err();
        assert!(error.to_string().contains("column count"), "{error}");
    }

    #[test]
    fn caps_are_enforced() {
        let many: Vec<Value> = (0..=MAX_BLOCKS)
            .map(|_| json!({ "type": "text", "text": "x" }))
            .collect();
        let error = validate(json!(many)).unwrap_err();
        assert!(error.to_string().contains("too many blocks"), "{error}");

        let long = "x".repeat(MAX_STRING_BYTES + 1);
        let error = validate(json!([{ "type": "text", "text": long }])).unwrap_err();
        assert!(error.to_string().contains("exceeds"), "{error}");

        let wide: Vec<Value> = (0..=MAX_TABLE_COLUMNS)
            .map(|i| json!(format!("c{i}")))
            .collect();
        let error =
            validate(json!([{ "type": "table", "columns": wide, "rows": [] }])).unwrap_err();
        assert!(error.to_string().contains("columns"), "{error}");
    }
}
