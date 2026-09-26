//! 以前のリスト形式の集合
//!
//! 集合を Map にする前のドキュメントは、エンティティを配列で持っている。読み取りはそのまま行い、
//! 書き込むときに同じトランザクションの中で Map に置き換える。置き換えはキーごとに 1 回きり。
//!
//! まだ端末間の同期は無いため、変換が 2 つの端末で同時に起きて一方の Map だけが残ることは
//! 考えなくてよい。同期を入れる前にすべてのドキュメントが変換済みになっている必要はない
//! （変換は書き込みのたびに確認される）が、同期の送信前に変換を済ませる方が安全。

use automerge::transaction::{Transactable, Transaction};
use automerge::{ObjId, ObjType, ROOT, ReadDoc};
use serde::de::DeserializeOwned;

use super::Collection;
use crate::errors::automerge_error::AutomergeError;
use crate::infrastructure::json::read::read_list;
use crate::infrastructure::json::reconcile::reconcile_map;

/// リスト `list` から `key` のエンティティを探す
pub(super) fn find<D: ReadDoc, T: DeserializeOwned>(
    doc: &D,
    list: &ObjId,
    collection: &Collection<T>,
    key: &str,
) -> Result<Option<T>, AutomergeError> {
    for item in read_list(doc, list) {
        let entity: T = serde_json::from_value(item)?;
        if collection.key_of(&entity) == key {
            return Ok(Some(entity));
        }
    }
    Ok(None)
}

/// リスト `list` を同じ内容の Map に置き換え、その Map を返す
///
/// 要素の JSON はそのまま移す（エンティティの型を通すのはキーを求めるときだけ）。
/// キーが重複していれば後の要素が残る。
pub(super) fn convert_to_map<T: DeserializeOwned>(
    tx: &mut Transaction,
    list: &ObjId,
    collection: &Collection<T>,
) -> Result<ObjId, AutomergeError> {
    let mut entries = serde_json::Map::new();
    for item in read_list(tx, list) {
        let entity: T = serde_json::from_value(item.clone())?;
        entries.insert(collection.key_of(&entity), item);
    }

    let map = tx.put_object(ROOT, collection.name(), ObjType::Map)?;
    reconcile_map(tx, &map, &entries)?;
    tracing::info!(
        collection = collection.name(),
        entries = entries.len(),
        "converted an Automerge collection from a list to a map"
    );
    Ok(map)
}
