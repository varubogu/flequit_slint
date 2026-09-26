//! 集合の読み書き（[`Document`] のメソッド）

use automerge::transaction::{Transactable, Transaction};
use automerge::{ObjId, ObjType, ROOT, ReadDoc, Value};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value as Json;

use super::{Collection, legacy};
use crate::errors::automerge_error::AutomergeError;
use crate::infrastructure::document::Document;
use crate::infrastructure::json::read::read_value;
use crate::infrastructure::json::reconcile::{reconcile_map, reconcile_prop};

impl Document {
    /// 集合の全エンティティを返す。集合が無ければ空
    pub async fn load_collection<T: DeserializeOwned>(
        &self,
        collection: &Collection<T>,
    ) -> Result<Vec<T>, AutomergeError> {
        let entries = self
            .handle
            .with_doc(|doc| -> Result<Vec<Json>, AutomergeError> {
                let Some((value, obj)) = doc.get(ROOT, collection.name())? else {
                    return Ok(Vec::new());
                };
                Ok(match read_value(doc, &value, &obj) {
                    Json::Object(entries) => entries.into_iter().map(|(_, entry)| entry).collect(),
                    Json::Array(items) => items,
                    _ => Vec::new(),
                })
            })?;
        entries
            .into_iter()
            // 以前の実装が削除の代わりに null を置いていた所は、無いものとして扱う
            .filter(|entry| !entry.is_null())
            .map(|entry| Ok(serde_json::from_value(entry)?))
            .collect()
    }

    /// `key` のエンティティを返す
    pub async fn load_entry<T: DeserializeOwned>(
        &self,
        collection: &Collection<T>,
        key: &str,
    ) -> Result<Option<T>, AutomergeError> {
        self.handle
            .with_doc(|doc| match doc.get(ROOT, collection.name())? {
                Some((Value::Object(ObjType::Map), map)) => {
                    let entry = match doc.get(&map, key)? {
                        Some((value, obj)) => read_value(doc, &value, &obj),
                        None => Json::Null,
                    };
                    if entry.is_null() {
                        return Ok(None);
                    }
                    Ok(Some(serde_json::from_value(entry)?))
                }
                Some((Value::Object(ObjType::List), list)) => {
                    legacy::find(doc, &list, collection, key)
                }
                _ => Ok(None),
            })
    }

    /// エンティティを追加する。既にあれば、変わったフィールドだけを書き換える
    pub async fn put_entry<T: Serialize + DeserializeOwned>(
        &self,
        collection: &Collection<T>,
        entity: &T,
    ) -> Result<(), AutomergeError> {
        self.put_entries(collection, std::slice::from_ref(entity))
            .await
    }

    /// [`Self::put_entry`] を 1 つの変更としてまとめて行う
    pub async fn put_entries<T: Serialize + DeserializeOwned>(
        &self,
        collection: &Collection<T>,
        entities: &[T],
    ) -> Result<(), AutomergeError> {
        let entries = to_entries(collection, entities)?;
        self.transact(|tx| {
            let map = collection_map(tx, collection)?;
            for (key, entry) in &entries {
                reconcile_prop(tx, &map, key, entry)?;
            }
            Ok(())
        })
    }

    /// `key` のエンティティを消す。あったかどうかを返す
    pub async fn delete_entry<T: DeserializeOwned>(
        &self,
        collection: &Collection<T>,
        key: &str,
    ) -> Result<bool, AutomergeError> {
        let deleted = self.delete_entries(collection, &[key.to_string()]).await?;
        Ok(deleted > 0)
    }

    /// `keys` のエンティティを 1 つの変更として消す。消した件数を返す
    pub async fn delete_entries<T: DeserializeOwned>(
        &self,
        collection: &Collection<T>,
        keys: &[String],
    ) -> Result<usize, AutomergeError> {
        self.transact(|tx| {
            let Some(map) = existing_collection_map(tx, collection)? else {
                return Ok(0);
            };
            let mut deleted = 0;
            for key in keys {
                if tx.get(&map, key.as_str())?.is_some() {
                    tx.delete(&map, key.as_str())?;
                    deleted += 1;
                }
            }
            Ok(deleted)
        })
    }

    /// `remove` が真を返すエンティティを 1 つの変更として消す。消した件数を返す
    pub async fn delete_entries_where<T: DeserializeOwned>(
        &self,
        collection: &Collection<T>,
        remove: impl Fn(&T) -> bool,
    ) -> Result<usize, AutomergeError> {
        self.transact(|tx| {
            let Some(map) = existing_collection_map(tx, collection)? else {
                return Ok(0);
            };
            let keys: Vec<String> = tx.keys(&map).collect();
            let mut deleted = 0;
            for key in keys {
                let entry = match tx.get(&map, key.as_str())? {
                    Some((value, obj)) => read_value(tx, &value, &obj),
                    None => continue,
                };
                let matched = !entry.is_null() && remove(&serde_json::from_value(entry)?);
                if matched {
                    tx.delete(&map, key.as_str())?;
                    deleted += 1;
                }
            }
            Ok(deleted)
        })
    }

    /// 集合を `entities` にする。変わったエンティティは差分だけを書き、無いものは消す
    ///
    /// 集合全体を読んでから書く処理に使うと、その間に追加されたエンティティまで消える。
    /// 個々の追加・更新・削除には [`Self::put_entry`] や [`Self::delete_entry`] を使うこと。
    pub async fn replace_collection<T: Serialize + DeserializeOwned>(
        &self,
        collection: &Collection<T>,
        entities: &[T],
    ) -> Result<(), AutomergeError> {
        let entries = to_entries(collection, entities)?;
        self.transact(|tx| {
            let map = collection_map(tx, collection)?;
            reconcile_map(tx, &map, &entries)?;
            Ok(())
        })
    }

    /// `write` を 1 つのトランザクションで行う。エラーならロールバックする
    fn transact<R>(
        &self,
        write: impl FnOnce(&mut Transaction) -> Result<R, AutomergeError>,
    ) -> Result<R, AutomergeError> {
        self.handle.with_doc_mut(|doc| {
            let mut tx = doc.transaction();
            let result = write(&mut tx)?;
            tx.commit();
            Ok(result)
        })
    }
}

fn to_entries<T: Serialize>(
    collection: &Collection<T>,
    entities: &[T],
) -> Result<serde_json::Map<String, Json>, AutomergeError> {
    entities
        .iter()
        .map(|entity| Ok((collection.key_of(entity), serde_json::to_value(entity)?)))
        .collect()
}

/// 集合の Map を返す。無ければ作る
fn collection_map<T: DeserializeOwned>(
    tx: &mut Transaction,
    collection: &Collection<T>,
) -> Result<ObjId, AutomergeError> {
    match existing_collection_map(tx, collection)? {
        Some(map) => Ok(map),
        None => Ok(tx.put_object(ROOT, collection.name(), ObjType::Map)?),
    }
}

/// 集合の Map を返す。無ければ `None`。以前のリスト形式なら Map に変換する
fn existing_collection_map<T: DeserializeOwned>(
    tx: &mut Transaction,
    collection: &Collection<T>,
) -> Result<Option<ObjId>, AutomergeError> {
    let list = match tx.get(ROOT, collection.name())? {
        Some((Value::Object(ObjType::Map), map)) => return Ok(Some(map)),
        Some((Value::Object(ObjType::List), list)) => list,
        _ => return Ok(None),
    };
    legacy::convert_to_map(tx, &list, collection).map(Some)
}
