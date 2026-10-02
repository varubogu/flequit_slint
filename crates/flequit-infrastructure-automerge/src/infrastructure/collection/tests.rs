use automerge::{ObjType, ROOT, ReadDoc, Value};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use super::Collection;
use crate::infrastructure::document::Document;
use crate::infrastructure::document_manager::{DocumentManager, DocumentType};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Item {
    id: String,
    title: String,
    done: bool,
}

const ITEMS: Collection<Item> = Collection::new("items", |item| item.id.clone());

fn item(id: &str, title: &str) -> Item {
    Item {
        id: id.to_string(),
        title: title.to_string(),
        done: false,
    }
}

/// 別々の端末に見立てた、独立したドキュメント
async fn document(dir: &TempDir) -> Document {
    let mut manager = DocumentManager::new(dir.path()).unwrap();
    manager.get_or_create(&DocumentType::User).await.unwrap()
}

fn ops(document: &Document) -> u64 {
    document.handle.with_doc(|doc| doc.stats().num_ops)
}

fn is_map(document: &Document, key: &str) -> bool {
    document.handle.with_doc(|doc| {
        matches!(
            doc.get(ROOT, key),
            Ok(Some((Value::Object(ObjType::Map), _)))
        )
    })
}

/// `from` の変更を `into` に取り込む（端末間の同期の代わり）
fn merge(into: &Document, from: &Document) {
    let mut from = from.handle.with_doc(|doc| doc.clone());
    into.handle
        .with_doc_mut(|doc| doc.merge(&mut from))
        .unwrap();
}

fn sorted(mut items: Vec<Item>) -> Vec<Item> {
    items.sort_by(|a, b| a.id.cmp(&b.id));
    items
}

#[tokio::test]
async fn entries_are_stored_as_a_map_and_read_back_by_key() {
    let dir = TempDir::new().unwrap();
    let document = document(&dir).await;

    document
        .put_entry(&ITEMS, &item("a", "first"))
        .await
        .unwrap();
    document
        .put_entry(&ITEMS, &item("b", "second"))
        .await
        .unwrap();

    assert!(is_map(&document, "items"));
    assert_eq!(
        document.load_entry(&ITEMS, "b").await.unwrap(),
        Some(item("b", "second"))
    );
    assert_eq!(document.load_entry(&ITEMS, "missing").await.unwrap(), None);
    assert_eq!(
        sorted(document.load_collection(&ITEMS).await.unwrap()),
        vec![item("a", "first"), item("b", "second")]
    );
}

#[tokio::test]
async fn updating_an_entry_writes_only_the_changed_field() {
    let dir = TempDir::new().unwrap();
    let document = document(&dir).await;
    document
        .put_entry(&ITEMS, &item("a", "first"))
        .await
        .unwrap();
    document
        .put_entry(&ITEMS, &item("b", "second"))
        .await
        .unwrap();
    let before = ops(&document);

    let mut renamed = item("a", "renamed");
    renamed.done = true;
    document.put_entry(&ITEMS, &renamed).await.unwrap();

    assert_eq!(
        ops(&document),
        before + 2,
        "one operation per changed field"
    );
    assert_eq!(
        document.load_entry(&ITEMS, "a").await.unwrap(),
        Some(renamed)
    );
}

#[tokio::test]
async fn saving_an_unchanged_entry_creates_no_change() {
    let dir = TempDir::new().unwrap();
    let document = document(&dir).await;
    document
        .put_entry(&ITEMS, &item("a", "first"))
        .await
        .unwrap();
    let heads = document.handle.with_doc(|doc| doc.get_heads());

    document
        .put_entry(&ITEMS, &item("a", "first"))
        .await
        .unwrap();

    assert_eq!(document.handle.with_doc(|doc| doc.get_heads()), heads);
}

#[tokio::test]
async fn a_legacy_list_is_read_as_is_and_becomes_a_map_on_the_first_write() {
    let dir = TempDir::new().unwrap();
    let document = document(&dir).await;
    // 以前の実装と同じく、集合を配列で保存する
    document
        .save_data("items", &vec![item("a", "first"), item("b", "second")])
        .await
        .unwrap();
    assert!(!is_map(&document, "items"));

    assert_eq!(
        document.load_collection(&ITEMS).await.unwrap(),
        vec![item("a", "first"), item("b", "second")]
    );
    assert_eq!(
        document.load_entry(&ITEMS, "b").await.unwrap(),
        Some(item("b", "second"))
    );

    document
        .put_entry(&ITEMS, &item("c", "third"))
        .await
        .unwrap();

    assert!(is_map(&document, "items"));
    assert_eq!(
        sorted(document.load_collection(&ITEMS).await.unwrap()),
        vec![item("a", "first"), item("b", "second"), item("c", "third")]
    );
}

#[tokio::test]
async fn deleting_removes_only_the_matching_entries() {
    let dir = TempDir::new().unwrap();
    let document = document(&dir).await;
    let mut done = item("c", "third");
    done.done = true;
    document
        .put_entries(&ITEMS, &[item("a", "first"), item("b", "second"), done])
        .await
        .unwrap();

    assert!(document.delete_entry(&ITEMS, "a").await.unwrap());
    assert!(!document.delete_entry(&ITEMS, "a").await.unwrap());
    assert_eq!(
        document
            .delete_entries_where(&ITEMS, |item| item.done)
            .await
            .unwrap(),
        1
    );

    assert_eq!(
        document.load_collection(&ITEMS).await.unwrap(),
        vec![item("b", "second")]
    );
}

#[tokio::test]
async fn deleting_from_a_missing_collection_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let document = document(&dir).await;

    assert!(!document.delete_entry(&ITEMS, "a").await.unwrap());

    assert!(document.handle.with_doc(|doc| doc.get_heads()).is_empty());
}

#[tokio::test]
async fn replacing_a_collection_removes_missing_entries_and_leaves_unchanged_ones_alone() {
    let dir = TempDir::new().unwrap();
    let document = document(&dir).await;
    document
        .put_entries(&ITEMS, &[item("a", "first"), item("b", "second")])
        .await
        .unwrap();
    let before = ops(&document);

    document
        .replace_collection(&ITEMS, &[item("a", "first")])
        .await
        .unwrap();

    // 削除は既存の操作に印を付けるだけで操作は増えない。a には何も書かない
    assert_eq!(ops(&document), before);
    assert_eq!(
        document.load_collection(&ITEMS).await.unwrap(),
        vec![item("a", "first")]
    );
}

#[tokio::test]
async fn a_null_left_by_an_earlier_delete_reads_as_missing() {
    let dir = TempDir::new().unwrap();
    let document = document(&dir).await;
    document
        .put_entry(&ITEMS, &item("a", "first"))
        .await
        .unwrap();
    document
        .save_data_at_path(&["items", "b"], &Option::<Item>::None)
        .await
        .unwrap();

    assert_eq!(
        document.load_collection(&ITEMS).await.unwrap(),
        vec![item("a", "first")]
    );
    assert_eq!(document.load_entry(&ITEMS, "b").await.unwrap(), None);
}

#[test]
fn collections_keep_the_keys_that_existing_documents_use() {
    use crate::infrastructure::accounts::account::ACCOUNTS;
    use crate::infrastructure::task_projects::{
        date_condition::DATE_CONDITIONS, member::MEMBERS, project_list_repository::PROJECTS,
        recurrence_rule::RECURRENCE_RULES, tag::TAGS, task::TASKS,
        task_assignments::TASK_ASSIGNMENTS, task_list::TASK_LISTS,
        task_recurrence::TASK_RECURRENCES, task_tag::TASK_TAGS,
        weekday_condition::WEEKDAY_CONDITIONS,
    };
    use crate::infrastructure::users::user::USERS;

    // 名前を変えると、保存済みのドキュメントの集合が読めなくなる
    let names = [
        ACCOUNTS.name(),
        USERS.name(),
        PROJECTS.name(),
        TASK_LISTS.name(),
        TASKS.name(),
        TAGS.name(),
        MEMBERS.name(),
        RECURRENCE_RULES.name(),
        DATE_CONDITIONS.name(),
        WEEKDAY_CONDITIONS.name(),
        TASK_TAGS.name(),
        TASK_ASSIGNMENTS.name(),
        TASK_RECURRENCES.name(),
    ];
    assert_eq!(
        names,
        [
            "accounts",
            "users",
            "projects",
            "task_lists",
            "tasks",
            "tags",
            "members",
            "recurrence_rules",
            "date_conditions",
            "weekday_conditions",
            "task_tags",
            "task_assignments",
            "task_recurrences",
        ]
    );
}

#[tokio::test]
async fn edits_to_different_entries_on_two_devices_both_survive_the_merge() {
    let (here_dir, there_dir) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let here = document(&here_dir).await;
    let there = document(&there_dir).await;
    here.put_entries(&ITEMS, &[item("a", "first"), item("b", "second")])
        .await
        .unwrap();
    merge(&there, &here);

    here.put_entry(&ITEMS, &item("a", "edited here"))
        .await
        .unwrap();
    there
        .put_entry(&ITEMS, &item("b", "edited there"))
        .await
        .unwrap();
    merge(&here, &there);

    assert_eq!(
        sorted(here.load_collection(&ITEMS).await.unwrap()),
        vec![item("a", "edited here"), item("b", "edited there")]
    );
}
