use std::path::{Path, PathBuf};

use crate::errors::automerge_error::AutomergeError;
use crate::infrastructure::document_manager::DocumentType;
use automerge::transaction::Transactable;
use automerge::{ObjType, ReadDoc, ScalarValue};
use automerge_repo::DocHandle;
use flequit_model::types::id_types::ProjectId;

/// JSON のスカラー値を Automerge のスカラー値にする。
fn json_scalar(value: &serde_json::Value) -> Result<ScalarValue, automerge::AutomergeError> {
    Ok(match value {
        serde_json::Value::Null => ScalarValue::Null,
        serde_json::Value::Bool(b) => ScalarValue::Boolean(*b),
        serde_json::Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => ScalarValue::Int(i),
            (None, Some(f)) => ScalarValue::F64(f),
            (None, None) => return Err(automerge::AutomergeError::InvalidOp(ObjType::Map)),
        },
        serde_json::Value::String(s) => ScalarValue::Str(s.as_str().into()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            return Err(automerge::AutomergeError::InvalidOp(ObjType::Map));
        }
    })
}

/// 既存の値が `scalar` と同じスカラーか。
fn is_same_scalar(existing: Option<&automerge::Value>, scalar: &ScalarValue) -> bool {
    matches!(existing, Some(automerge::Value::Scalar(current)) if current.as_ref() == scalar)
}

#[derive(Debug, Clone)]
pub struct Document {
    pub base_path: PathBuf,
    pub doc_type: DocumentType,
    pub handle: DocHandle,
}

impl Document {
    pub fn new(base_path: PathBuf, doc_type: DocumentType, doc_handle: DocHandle) -> Document {
        Self {
            base_path,
            doc_type,
            handle: doc_handle,
        }
    }

    /// プロジェクトIDを取得（Project型の場合のみ）
    pub fn project_id(&self) -> Option<ProjectId> {
        self.doc_type.project_id()
    }
    /// ドキュメントにデータを保存
    pub async fn save_data<T: serde::Serialize>(
        &self,
        key: &str,
        value: &T,
    ) -> Result<(), AutomergeError> {
        // デバッグモード時のJSON出力（保存前）
        #[cfg(debug_assertions)]
        {
            let json_value = serde_json::to_value(value)
                .map_err(|e| AutomergeError::SerializationError(e.to_string()))?;
            tracing::debug!(
                "Automerge save - doc_type: {:?}, key: {} -> JSON: {}",
                self.doc_type,
                key,
                serde_json::to_string_pretty(&json_value)
                    .unwrap_or_else(|_| "Invalid JSON".to_string())
            );
        }

        self.save_data_at_path(&[key], value).await
    }

    /// 指定されたパスにデータを保存
    pub async fn save_data_at_path<T: serde::Serialize>(
        &self,
        path: &[&str],
        value: &T,
    ) -> Result<(), AutomergeError> {
        let doc = self;

        // JSON形式でシリアライズしてから保存
        let json_value = serde_json::to_value(value)
            .map_err(|e| AutomergeError::SerializationError(e.to_string()))?;

        doc.handle.with_doc_mut(|doc| {
            let mut tx = doc.transaction();

            if path.is_empty() {
                return Err(AutomergeError::InvalidOperation(
                    "Empty path not allowed".to_string(),
                ));
            }

            if path.len() == 1 {
                // 単一キーの場合は従来通り
                self.put_json_value(&mut tx, &automerge::ROOT, path[0], &json_value)
                    .map_err(|e| AutomergeError::AutomergeError(e.to_string()))?;
            } else {
                // パス指定の場合は段階的にオブジェクトを辿る
                let target_obj = self
                    .get_or_create_nested_object(&mut tx, &automerge::ROOT, &path[..path.len() - 1])
                    .map_err(|e| AutomergeError::AutomergeError(e.to_string()))?;
                self.put_json_value(&mut tx, &target_obj, path[path.len() - 1], &json_value)
                    .map_err(|e| AutomergeError::AutomergeError(e.to_string()))?;
            }

            tx.commit();

            // デバッグモード時のJSON出力
            #[cfg(debug_assertions)]
            {
                let path_str = path.join("/");
                tracing::debug!(
                    "Automerge save - path: {} -> JSON: {}",
                    path_str,
                    serde_json::to_string_pretty(&json_value)
                        .unwrap_or_else(|_| "Invalid JSON".to_string())
                );
            }

            Ok(())
        })
    }

    /// ドキュメントからデータを読み込み
    pub async fn load_data<T: serde::de::DeserializeOwned + serde::Serialize>(
        &self,
        key: &str,
    ) -> Result<Option<T>, AutomergeError> {
        let result = self.load_data_at_path(&[key]).await;

        // デバッグモード時のJSON出力（読み込み結果）
        #[cfg(debug_assertions)]
        {
            match &result {
                Ok(Some(data)) => {
                    if let Ok(json_value) = serde_json::to_value(data) {
                        tracing::debug!(
                            "Automerge load - doc_type: {:?}, key: {} <- JSON: {}",
                            self.doc_type,
                            key,
                            serde_json::to_string_pretty(&json_value)
                                .unwrap_or_else(|_| "Invalid JSON".to_string())
                        );
                    }
                }
                Ok(None) => {
                    tracing::debug!(
                        "Automerge load - doc_type: {:?}, key: {} <- NOT FOUND",
                        self.doc_type,
                        key
                    );
                }
                Err(e) => {
                    tracing::debug!(
                        "Automerge load - doc_type: {:?}, key: {} <- ERROR: {:?}",
                        self.doc_type,
                        key,
                        e
                    );
                }
            }
        }

        result
    }

    /// 指定されたパスからデータを読み込み
    pub async fn load_data_at_path<T: serde::de::DeserializeOwned + serde::Serialize>(
        &self,
        path: &[&str],
    ) -> Result<Option<T>, AutomergeError> {
        let doc = self;

        doc.handle.with_doc(|doc| {
            if path.is_empty() {
                return Err(AutomergeError::InvalidOperation(
                    "Empty path not allowed".to_string(),
                ));
            }

            // パスを辿ってオブジェクトを取得
            let mut current_obj = automerge::ROOT;
            for (i, &key) in path.iter().enumerate() {
                match doc.get(&current_obj, key) {
                    Ok(Some((value, obj_id))) => {
                        if i == path.len() - 1 {
                            // 最後のキーに到達した場合、値を返す
                            let json_value =
                                self.value_to_json_value_with_objid(doc, &value, &obj_id);
                            if json_value == serde_json::Value::Null {
                                return Ok(None);
                            }

                            // デバッグモード時のJSON出力（読み込み時）
                            #[cfg(debug_assertions)]
                            {
                                let path_str = path.join("/");
                                tracing::debug!(
                                    "Automerge load - path: {} <- JSON: {}",
                                    path_str,
                                    serde_json::to_string_pretty(&json_value)
                                        .unwrap_or_else(|_| "Invalid JSON".to_string())
                                );
                            }

                            let result: T = serde_json::from_value(json_value)
                                .map_err(|e| AutomergeError::SerializationError(e.to_string()))?;
                            return Ok(Some(result));
                        } else {
                            // 中間のオブジェクト、次に進む
                            current_obj = obj_id;
                        }
                    }
                    Ok(None) => {
                        #[cfg(debug_assertions)]
                        {
                            let path_str = path.join("/");
                            tracing::debug!("Automerge load - path: {} <- NOT FOUND", path_str);
                        }
                        return Ok(None);
                    }
                    Err(_) => {
                        #[cfg(debug_assertions)]
                        {
                            let path_str = path.join("/");
                            tracing::debug!("Automerge load - path: {} <- ERROR", path_str);
                        }
                        return Ok(None);
                    }
                }
            }
            Ok(None)
        })
    }

    /// 特定のキーの値を更新
    pub async fn update_value(&self, key: &str, value: &str) -> Result<(), AutomergeError> {
        let doc = self;

        doc.handle.with_doc_mut(|doc| {
            let mut tx = doc.transaction();

            // シンプルなルートレベルキーのみサポート（ネストは後で実装）
            tx.put(automerge::ROOT, key, value)
                .map_err(|e| AutomergeError::AutomergeError(e.to_string()))?;
            tx.commit();
            Ok(())
        })
    }

    /// ドキュメントの全データをJSONとして取得
    pub async fn export_document_as_json(&self) -> Result<serde_json::Value, AutomergeError> {
        let doc = self;

        doc.handle.with_doc(|doc| {
            let root_value = match doc.get(&automerge::ROOT, "dummy_root_key") {
                Ok(Some((value, obj_id))) => {
                    // ダミーキーで取得した場合の処理
                    self.value_to_json_value_with_objid(doc, &value, &obj_id)
                }
                _ => {
                    // ルートオブジェクト全体を読み取る
                    self.read_map_object(doc, &automerge::ROOT)
                }
            };
            Ok(root_value)
        })
    }

    /// ドキュメントの状態をJSONファイルに出力
    pub async fn export_json<P: AsRef<Path>>(
        &self,
        output_path: P,
        description: Option<&str>,
    ) -> Result<(), AutomergeError> {
        let json_data = self.export_document_as_json().await?;

        // メタデータを含むJSONを作成
        let export_data = serde_json::json!({
            "metadata": {
                "document_type": format!("{:?}", self.doc_type),
                "filename": self.doc_type.filename(),
                "exported_at": chrono::Utc::now().to_rfc3339(),
                "description": description.unwrap_or("Document state export")
            },
            "document_data": json_data
        });

        let json_string = serde_json::to_string_pretty(&export_data)
            .map_err(|e| AutomergeError::SerializationError(e.to_string()))?;

        std::fs::write(output_path, json_string)
            .map_err(|e| AutomergeError::IOError(e.to_string()))?;

        Ok(())
    }

    /// Automergeドキュメントの変更履歴を段階的にJSONで出力
    pub async fn export_document_changes_history<P: AsRef<Path>>(
        &self,
        output_dir: P,
        description: Option<&str>,
    ) -> Result<(), AutomergeError> {
        let output_dir = output_dir.as_ref();
        std::fs::create_dir_all(output_dir).map_err(|e| AutomergeError::IOError(e.to_string()))?;

        let doc = self;
        let changes_history = doc.handle.with_doc(|doc| {
            let mut history = Vec::new();
            let _heads = doc.get_heads();

            // 各変更ポイントでのドキュメント状態を取得
            for (change_index, change) in doc.get_changes(&[]).iter().enumerate() {
                let change_hash = change.hash();

                // この変更までのドキュメント状態を取得
                let doc_at_change = {
                    let mut temp_doc = automerge::Automerge::new();
                    let changes: Vec<_> = doc
                        .get_changes(&[])
                        .into_iter()
                        .take(change_index + 1)
                        .collect();
                    temp_doc
                        .apply_changes(changes)
                        .map_err(|e| AutomergeError::AutomergeError(e.to_string()))?;
                    temp_doc
                };

                let root_value = self.read_map_object(&doc_at_change, &automerge::ROOT);

                history.push(serde_json::json!({
                    "change_index": change_index,
                    "change_hash": format!("{:?}", change_hash),
                    "actor": format!("{:?}", change.actor_id()),
                    "timestamp": change.timestamp(),
                    "message": change.message().map_or("", |v| v),
                    "document_state": root_value
                }));
            }

            // 最新状態も追加
            let current_state = self.read_map_object(doc, &automerge::ROOT);
            history.push(serde_json::json!({
                "change_index": "current",
                "change_hash": "HEAD",
                "actor": "current",
                "timestamp": chrono::Utc::now().timestamp(),
                "message": "Current state",
                "document_state": current_state
            }));

            Ok::<Vec<serde_json::Value>, AutomergeError>(history)
        })?;

        // 各変更を個別ファイルに出力
        for (index, change_data) in changes_history.iter().enumerate() {
            let filename = if index == changes_history.len() - 1 {
                "current_state.json".to_string()
            } else {
                format!("change_{:03}.json", index)
            };

            let export_data = serde_json::json!({
                "metadata": {
                    "document_type": format!("{:?}", self.doc_type),
                    "filename": self.doc_type.filename(),
                    "exported_at": chrono::Utc::now().to_rfc3339(),
                    "description": description.unwrap_or("Automerge change history export"),
                    "change_sequence": index
                },
                "change_data": change_data
            });

            let json_string = serde_json::to_string_pretty(&export_data)
                .map_err(|e| AutomergeError::SerializationError(e.to_string()))?;

            let file_path = output_dir.join(filename);
            std::fs::write(file_path, json_string)
                .map_err(|e| AutomergeError::IOError(e.to_string()))?;
        }

        // サマリーファイルも作成
        let summary_data = serde_json::json!({
            "metadata": {
                "document_type": format!("{:?}", self.doc_type),
                "filename": self.doc_type.filename(),
                "exported_at": chrono::Utc::now().to_rfc3339(),
                "description": description.unwrap_or("Automerge change history summary"),
                "total_changes": changes_history.len() - 1,
                "summary_type": "change_history"
            },
            "changes_summary": changes_history.iter().enumerate().map(|(index, change)| {
                serde_json::json!({
                    "index": index,
                    "change_hash": change["change_hash"],
                    "timestamp": change["timestamp"],
                    "message": change["message"],
                    "filename": if index == changes_history.len() - 1 {
                        "current_state.json".to_string()
                    } else {
                        format!("change_{:03}.json", index)
                    }
                })
            }).collect::<Vec<_>>()
        });

        let summary_json = serde_json::to_string_pretty(&summary_data)
            .map_err(|e| AutomergeError::SerializationError(e.to_string()))?;

        std::fs::write(output_dir.join("changes_summary.json"), summary_json)
            .map_err(|e| AutomergeError::IOError(e.to_string()))?;

        Ok(())
    }

    /// ネストしたオブジェクトを取得または作成するヘルパー
    fn get_or_create_nested_object(
        &self,
        tx: &mut automerge::transaction::Transaction,
        root_obj: &automerge::ObjId,
        path: &[&str],
    ) -> Result<automerge::ObjId, automerge::AutomergeError> {
        let mut current_obj = root_obj.clone();

        for &key in path {
            // 現在のオブジェクトから次のオブジェクトを取得
            match tx.get(&current_obj, key)? {
                Some((automerge::Value::Object(_), obj_id)) => {
                    // オブジェクトが既に存在する場合
                    current_obj = obj_id;
                }
                Some((_, _)) => {
                    // 既存の値がオブジェクト以外の場合はエラー
                    return Err(automerge::AutomergeError::InvalidOp(
                        automerge::ObjType::Map,
                    ));
                }
                None => {
                    // オブジェクトが存在しない場合は作成
                    current_obj = tx.put_object(&current_obj, key, automerge::ObjType::Map)?;
                }
            }
        }

        Ok(current_obj)
    }

    /// JSON ValueをAutomergeに書き込むヘルパー
    ///
    /// 既存の値との差分だけを書く。リポジトリは保存のたびにエンティティ配列全体を
    /// 渡してくるため、毎回オブジェクトを作り直すと変更履歴が保存回数に比例して
    /// 膨らみ、書き込みそのものが次第に遅くなる（数件のタスクで 1 回数秒）。
    /// 同じ値は書かず、マップとリストは既存オブジェクトをその場で更新する。
    fn put_json_value(
        &self,
        tx: &mut automerge::transaction::Transaction,
        obj: &automerge::ObjId,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), automerge::AutomergeError> {
        let existing = tx.get(obj, key)?;
        match value {
            serde_json::Value::Array(arr) => {
                let list_id = match existing {
                    Some((automerge::Value::Object(ObjType::List), id)) => id,
                    _ => tx.put_object(obj, key, ObjType::List)?,
                };
                self.reconcile_list(tx, &list_id, arr)
            }
            serde_json::Value::Object(map) => {
                let map_id = match existing {
                    Some((automerge::Value::Object(ObjType::Map), id)) => id,
                    _ => tx.put_object(obj, key, ObjType::Map)?,
                };
                self.reconcile_map(tx, &map_id, map)
            }
            scalar => {
                let scalar = json_scalar(scalar)?;
                if !is_same_scalar(existing.as_ref().map(|(value, _)| value), &scalar) {
                    tx.put(obj, key, scalar)?;
                }
                Ok(())
            }
        }
    }

    /// 既存のマップを `map` と同じ内容に揃える。
    fn reconcile_map(
        &self,
        tx: &mut automerge::transaction::Transaction,
        map_id: &automerge::ObjId,
        map: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), automerge::AutomergeError> {
        let stale: Vec<String> = tx
            .keys(map_id)
            .filter(|key| !map.contains_key(key))
            .collect();
        for key in stale {
            tx.delete(map_id, key.as_str())?;
        }
        for (k, v) in map.iter() {
            self.put_json_value(tx, map_id, k, v)?;
        }
        Ok(())
    }

    /// 既存のリストを `arr` と同じ内容に揃える。
    ///
    /// 位置ごとに突き合わせる。保存は末尾への追加か既存要素の更新がほとんどなので、
    /// 要素の対応付けを探すより単純で十分に差分が小さい。
    fn reconcile_list(
        &self,
        tx: &mut automerge::transaction::Transaction,
        list_id: &automerge::ObjId,
        arr: &[serde_json::Value],
    ) -> Result<(), automerge::AutomergeError> {
        let length = tx.length(list_id);
        for index in (arr.len()..length).rev() {
            tx.delete(list_id, index)?;
        }

        for (index, item) in arr.iter().enumerate() {
            if index >= length {
                self.put_json_value_at_index(tx, list_id, index, item)?;
                continue;
            }
            let existing = tx.get(list_id, index)?;
            match item {
                serde_json::Value::Array(nested) => match existing {
                    Some((automerge::Value::Object(ObjType::List), id)) => {
                        self.reconcile_list(tx, &id, nested)?;
                    }
                    _ => {
                        let id = tx.put_object(list_id, index, ObjType::List)?;
                        self.reconcile_list(tx, &id, nested)?;
                    }
                },
                serde_json::Value::Object(map) => match existing {
                    Some((automerge::Value::Object(ObjType::Map), id)) => {
                        self.reconcile_map(tx, &id, map)?;
                    }
                    _ => {
                        let id = tx.put_object(list_id, index, ObjType::Map)?;
                        self.reconcile_map(tx, &id, map)?;
                    }
                },
                scalar => {
                    let scalar = json_scalar(scalar)?;
                    if !is_same_scalar(existing.as_ref().map(|(value, _)| value), &scalar) {
                        tx.put(list_id, index, scalar)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// 配列インデックスに値を設定するヘルパー
    fn put_json_value_at_index(
        &self,
        tx: &mut automerge::transaction::Transaction,
        list_obj: &automerge::ObjId,
        index: usize,
        value: &serde_json::Value,
    ) -> Result<(), automerge::AutomergeError> {
        match value {
            serde_json::Value::Null => {
                tx.insert(list_obj, index, ScalarValue::Null)?;
            }
            serde_json::Value::Bool(b) => {
                tx.insert(list_obj, index, *b)?;
            }
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    tx.insert(list_obj, index, i)?;
                } else if let Some(f) = n.as_f64() {
                    tx.insert(list_obj, index, f)?;
                } else {
                    return Err(automerge::AutomergeError::InvalidOp(ObjType::Map));
                }
            }
            serde_json::Value::String(s) => {
                tx.insert(list_obj, index, s.as_str())?;
            }
            serde_json::Value::Array(arr) => {
                let nested_list_id = tx.insert_object(list_obj, index, ObjType::List)?;
                for (i, item) in arr.iter().enumerate() {
                    self.put_json_value_at_index(tx, &nested_list_id, i, item)?;
                }
            }
            serde_json::Value::Object(map) => {
                let nested_map_id = tx.insert_object(list_obj, index, ObjType::Map)?;
                for (k, v) in map.iter() {
                    self.put_json_value(tx, &nested_map_id, k, v)?;
                }
            }
        }
        Ok(())
    }

    /// ValueをJSON Valueに変換するヘルパー（オブジェクトID付き版）
    fn value_to_json_value_with_objid<D: ReadDoc>(
        &self,
        doc: &D,
        value: &automerge::Value,
        obj_id: &automerge::ObjId,
    ) -> serde_json::Value {
        match value {
            automerge::Value::Scalar(scalar) => self.scalar_to_json_value(scalar),
            automerge::Value::Object(obj_type) => match obj_type {
                automerge::ObjType::Map | automerge::ObjType::Table => {
                    self.read_map_object(doc, obj_id)
                }
                automerge::ObjType::List => self.read_list_object(doc, obj_id),
                automerge::ObjType::Text => self.read_text_object(doc, obj_id),
            },
        }
    }

    /// Mapオブジェクトからデータを読み取る
    fn read_map_object<D: ReadDoc>(&self, doc: &D, obj_id: &automerge::ObjId) -> serde_json::Value {
        let mut map = serde_json::Map::new();

        // Mapのすべてのキーと値を取得
        for key in doc.keys(obj_id.clone()) {
            if let Ok(Some((value, nested_obj_id))) = doc.get(obj_id, &key) {
                let json_value = self.value_to_json_value_with_objid(doc, &value, &nested_obj_id);
                map.insert(key, json_value);
            }
        }

        serde_json::Value::Object(map)
    }

    /// Listオブジェクトからデータを読み取る
    fn read_list_object<D: ReadDoc>(
        &self,
        doc: &D,
        obj_id: &automerge::ObjId,
    ) -> serde_json::Value {
        let mut array = Vec::new();

        // Listの長さを取得
        let length = doc.length(obj_id.clone());

        // 各インデックスの値を取得
        for i in 0..length {
            if let Ok(Some((value, nested_obj_id))) = doc.get(obj_id, i) {
                let json_value = self.value_to_json_value_with_objid(doc, &value, &nested_obj_id);
                array.push(json_value);
            }
        }

        serde_json::Value::Array(array)
    }

    /// Textオブジェクトからデータを読み取る
    fn read_text_object<D: ReadDoc>(
        &self,
        doc: &D,
        obj_id: &automerge::ObjId,
    ) -> serde_json::Value {
        // Textオブジェクトをstringとして読み取る
        match doc.text(obj_id.clone()) {
            Ok(text) => serde_json::Value::String(text),
            Err(_) => serde_json::Value::String("".to_string()),
        }
    }

    /// ScalarValueをJSON Valueに変換するヘルパー
    fn scalar_to_json_value(&self, value: &ScalarValue) -> serde_json::Value {
        match value {
            ScalarValue::Null => serde_json::Value::Null,
            ScalarValue::Boolean(b) => serde_json::Value::Bool(*b),
            ScalarValue::Int(i) => serde_json::Value::Number((*i).into()),
            ScalarValue::F64(f) => serde_json::Value::Number(
                serde_json::Number::from_f64(*f).unwrap_or_else(|| serde_json::Number::from(0)),
            ),
            ScalarValue::Str(s) => serde_json::Value::String(s.to_string()),
            ScalarValue::Bytes(b) => {
                // バイト列は文字列として表現
                serde_json::Value::String(format!("bytes[{}]", b.len()))
            }
            ScalarValue::Timestamp(ts) => serde_json::Value::Number((*ts).into()),
            ScalarValue::Counter(_c) => serde_json::Value::Number(0.into()), // Counterの値は直接取得できないため0とする
            ScalarValue::Uint(u) => serde_json::Value::Number((*u).into()),
            ScalarValue::Unknown { .. } => serde_json::Value::Null, // 未知の型はNullとする
        }
    }

    /// ドキュメント全体をロード（互換性メソッド）
    pub async fn load<T: serde::de::DeserializeOwned + serde::Serialize>(
        &mut self,
    ) -> Result<Option<T>, AutomergeError> {
        self.load_data("root").await
    }

    /// ドキュメント全体を保存（互換性メソッド）
    pub async fn save<T: serde::Serialize>(&mut self, value: &T) -> Result<(), AutomergeError> {
        self.save_data("root", value).await
    }
}
