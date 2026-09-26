//! JSON の値を Automerge ドキュメントへ差分で書く
//!
//! 値の変わった所だけに操作を積む。変わっていなければ何も書かないので、同じ内容の保存は
//! 変更（change）を作らない。丸ごと置き換えると、保存のたびに全フィールド分の操作が
//! 履歴に積もり、別の端末で同時に更新した他のフィールドも上書きしてしまう。

use automerge::transaction::{Transactable, Transaction};
use automerge::{AutomergeError, ObjId, ObjType, ReadDoc, ScalarValue, Value};
use serde_json::Value as Json;

use super::read::{read_value, scalar_to_json};

/// Map `obj` の `key` を `value` に合わせる
///
/// - オブジェクト: 既存の Map があればその中を再帰的に合わせる（ない値のキーは消す）
/// - 配列: 内容が違うときだけ新しい List に置き換える。要素を識別するキーを持たない
///   ため要素単位では合わせられない。同時に編集されうる集合は配列にせず
///   [`crate::infrastructure::collection::Collection`] に置くこと
/// - スカラー: 値が違うときだけ書く
pub fn reconcile_prop(
    tx: &mut Transaction,
    obj: &ObjId,
    key: &str,
    value: &Json,
) -> Result<(), AutomergeError> {
    match value {
        Json::Object(fields) => {
            let existing_map = match tx.get(obj, key)? {
                Some((Value::Object(ObjType::Map), map)) => Some(map),
                _ => None,
            };
            let map = match existing_map {
                Some(map) => map,
                None => tx.put_object(obj, key, ObjType::Map)?,
            };
            reconcile_map(tx, &map, fields)
        }
        Json::Array(items) => {
            let unchanged = match tx.get(obj, key)? {
                Some((current, current_obj)) => read_value(tx, &current, &current_obj) == *value,
                None => false,
            };
            if unchanged {
                return Ok(());
            }
            let list = tx.put_object(obj, key, ObjType::List)?;
            for (index, item) in items.iter().enumerate() {
                insert_value(tx, &list, index, item)?;
            }
            Ok(())
        }
        scalar => {
            let unchanged = match tx.get(obj, key)? {
                Some((Value::Scalar(current), _)) => scalar_to_json(&current) == *scalar,
                _ => false,
            };
            if unchanged {
                return Ok(());
            }
            tx.put(obj, key, to_scalar(scalar)?)
        }
    }
}

/// Map `map` を `fields` に合わせる。`fields` にないキーは消す
pub fn reconcile_map(
    tx: &mut Transaction,
    map: &ObjId,
    fields: &serde_json::Map<String, Json>,
) -> Result<(), AutomergeError> {
    let stale: Vec<String> = tx
        .keys(map)
        .filter(|key| !fields.contains_key(key))
        .collect();
    for key in stale {
        tx.delete(map, key.as_str())?;
    }
    for (key, value) in fields {
        reconcile_prop(tx, map, key, value)?;
    }
    Ok(())
}

/// List `list` の `index` に `value` を挿入する
fn insert_value(
    tx: &mut Transaction,
    list: &ObjId,
    index: usize,
    value: &Json,
) -> Result<(), AutomergeError> {
    match value {
        Json::Object(fields) => {
            let map = tx.insert_object(list, index, ObjType::Map)?;
            reconcile_map(tx, &map, fields)
        }
        Json::Array(items) => {
            let nested = tx.insert_object(list, index, ObjType::List)?;
            for (nested_index, item) in items.iter().enumerate() {
                insert_value(tx, &nested, nested_index, item)?;
            }
            Ok(())
        }
        scalar => tx.insert(list, index, to_scalar(scalar)?),
    }
}

/// オブジェクト・配列以外の JSON をスカラー値にする
fn to_scalar(value: &Json) -> Result<ScalarValue, AutomergeError> {
    match value {
        Json::Null => Ok(ScalarValue::Null),
        Json::Bool(b) => Ok(ScalarValue::Boolean(*b)),
        Json::Number(n) => n
            .as_i64()
            .map(ScalarValue::Int)
            .or_else(|| n.as_f64().map(ScalarValue::F64))
            .ok_or(AutomergeError::InvalidOp(ObjType::Map)),
        Json::String(s) => Ok(ScalarValue::Str(s.as_str().into())),
        Json::Array(_) | Json::Object(_) => Err(AutomergeError::InvalidOp(ObjType::Map)),
    }
}

#[cfg(test)]
mod tests {
    use automerge::{Automerge, ROOT};
    use serde_json::json;

    use super::*;
    use crate::infrastructure::json::read::read_map;

    fn write(doc: &mut Automerge, key: &str, value: &Json) {
        let mut tx = doc.transaction();
        reconcile_prop(&mut tx, &ROOT, key, value).unwrap();
        tx.commit();
    }

    fn ops(doc: &Automerge) -> u64 {
        doc.stats().num_ops
    }

    #[test]
    fn writing_the_same_value_again_adds_no_operations() {
        let mut doc = Automerge::new();
        let task = json!({"id": "t1", "title": "a", "done": false, "tags": ["x", "y"], "rule": {"every": 2}});
        write(&mut doc, "task", &task);
        let before = ops(&doc);
        let heads = doc.get_heads();

        write(&mut doc, "task", &task);

        assert_eq!(ops(&doc), before);
        assert_eq!(
            doc.get_heads(),
            heads,
            "an unchanged save should not create a change"
        );
    }

    #[test]
    fn only_the_changed_field_is_written() {
        let mut doc = Automerge::new();
        write(
            &mut doc,
            "task",
            &json!({"id": "t1", "title": "a", "done": false}),
        );
        let before = ops(&doc);

        write(
            &mut doc,
            "task",
            &json!({"id": "t1", "title": "b", "done": false}),
        );

        assert_eq!(ops(&doc), before + 1);
        assert_eq!(read_map(&doc, &ROOT)["task"]["title"], "b");
    }

    #[test]
    fn a_field_missing_from_the_new_value_is_removed() {
        let mut doc = Automerge::new();
        write(&mut doc, "task", &json!({"id": "t1", "note": "x"}));

        write(&mut doc, "task", &json!({"id": "t1"}));

        assert_eq!(read_map(&doc, &ROOT)["task"], json!({"id": "t1"}));
    }

    #[test]
    fn values_round_trip_through_the_document() {
        let mut doc = Automerge::new();
        let value = json!({
            "null": null, "bool": true, "int": -3, "float": 1.5, "text": "あ",
            "list": [1, {"nested": [true]}], "map": {"deep": {"x": 1}}
        });

        write(&mut doc, "value", &value);

        assert_eq!(read_map(&doc, &ROOT)["value"], value);
    }

    #[test]
    fn edits_to_different_fields_on_two_devices_both_survive_the_merge() {
        let mut here = Automerge::new();
        write(&mut here, "task", &json!({"title": "a", "done": false}));
        let mut there = here.fork();

        write(&mut here, "task", &json!({"title": "b", "done": false}));
        write(&mut there, "task", &json!({"title": "a", "done": true}));
        here.merge(&mut there).unwrap();

        assert_eq!(
            read_map(&here, &ROOT)["task"],
            json!({"title": "b", "done": true})
        );
    }
}
