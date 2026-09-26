//! Automerge の値を JSON として読む

use automerge::{ObjId, ObjType, ReadDoc, ScalarValue, Value};
use serde_json::Value as Json;

/// `value`（`obj` はそのオブジェクト ID）を JSON に変換する
pub fn read_value<D: ReadDoc>(doc: &D, value: &Value, obj: &ObjId) -> Json {
    match value {
        Value::Scalar(scalar) => scalar_to_json(scalar),
        Value::Object(ObjType::Map | ObjType::Table) => read_map(doc, obj),
        Value::Object(ObjType::List) => Json::Array(read_list(doc, obj)),
        Value::Object(ObjType::Text) => read_text(doc, obj),
    }
}

/// Map オブジェクトを JSON のオブジェクトとして読む
pub fn read_map<D: ReadDoc>(doc: &D, obj: &ObjId) -> Json {
    let mut map = serde_json::Map::new();
    for key in doc.keys(obj) {
        if let Ok(Some((value, nested))) = doc.get(obj, key.as_str()) {
            map.insert(key, read_value(doc, &value, &nested));
        }
    }
    Json::Object(map)
}

/// List オブジェクトの要素を読む
pub fn read_list<D: ReadDoc>(doc: &D, obj: &ObjId) -> Vec<Json> {
    (0..doc.length(obj))
        .filter_map(|index| match doc.get(obj, index) {
            Ok(Some((value, nested))) => Some(read_value(doc, &value, &nested)),
            _ => None,
        })
        .collect()
}

/// Text オブジェクトを文字列として読む
fn read_text<D: ReadDoc>(doc: &D, obj: &ObjId) -> Json {
    Json::String(doc.text(obj).unwrap_or_default())
}

/// スカラー値を JSON に変換する
pub fn scalar_to_json(value: &ScalarValue) -> Json {
    match value {
        ScalarValue::Null => Json::Null,
        ScalarValue::Boolean(b) => Json::Bool(*b),
        ScalarValue::Int(i) => Json::Number((*i).into()),
        ScalarValue::F64(f) => Json::Number(
            serde_json::Number::from_f64(*f).unwrap_or_else(|| serde_json::Number::from(0)),
        ),
        ScalarValue::Str(s) => Json::String(s.to_string()),
        // バイト列は文字列として表現
        ScalarValue::Bytes(b) => Json::String(format!("bytes[{}]", b.len())),
        ScalarValue::Timestamp(ts) => Json::Number((*ts).into()),
        // Counterの値は直接取得できないため0とする
        ScalarValue::Counter(_) => Json::Number(0.into()),
        ScalarValue::Uint(u) => Json::Number((*u).into()),
        // 未知の型はNullとする
        ScalarValue::Unknown { .. } => Json::Null,
    }
}
