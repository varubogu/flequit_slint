//! 保存が既存の値との差分だけを書くことの確認
//!
//! リポジトリは保存のたびにエンティティ配列全体を渡す。これを毎回作り直すと
//! 変更履歴が保存回数に比例して膨らみ、書き込みが次第に遅くなる。

use serde_json::{Value, json};

use flequit_infrastructure_automerge::infrastructure::document_manager::{
    DocumentManager, DocumentType,
};
use flequit_testing::TestPathGenerator;

fn tasks(count: usize, completed_index: Option<usize>) -> Value {
    Value::Array(
        (0..count)
            .map(|i| {
                json!({
                    "id": format!("task-{i}"),
                    "title": format!("Task {i}"),
                    "status": if Some(i) == completed_index { "completed" } else { "not_started" },
                    "priority": 3,
                    "description": null,
                    "reminders": ["2026-09-15T00:00:00Z"],
                    "tag_ids": [],
                })
            })
            .collect(),
    )
}

/// 直前の保存から増えた変更と、その操作数の合計。
fn ops_since(
    handle: &automerge_repo::DocHandle,
    heads: &[automerge::ChangeHash],
) -> (usize, usize) {
    handle.with_doc(|doc| {
        let changes = doc.get_changes(heads);
        (
            changes.len(),
            changes.iter().map(|change| change.len()).sum(),
        )
    })
}

#[tokio::test]
async fn saving_writes_only_what_changed() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir_path = TestPathGenerator::generate_test_dir(file!(), "write_diff");
    std::fs::create_dir_all(&temp_dir_path)?;
    let mut manager = DocumentManager::new(&temp_dir_path)?;
    let doc_type = DocumentType::Project("11111111-1111-1111-1111-111111111111".into());

    manager
        .save_data(&doc_type, "tasks", &tasks(5, None))
        .await?;
    let document = manager.get_or_create(&doc_type).await?;
    let heads = document.handle.with_doc(|doc| doc.get_heads());

    // 同じ内容をもう一度保存しても何も書かない
    manager
        .save_data(&doc_type, "tasks", &tasks(5, None))
        .await?;
    assert_eq!(ops_since(&document.handle, &heads), (0, 0));

    // 1 件のステータス変更は 1 操作だけ
    manager
        .save_data(&doc_type, "tasks", &tasks(5, Some(2)))
        .await?;
    assert_eq!(ops_since(&document.handle, &heads), (1, 1));
    let heads = document.handle.with_doc(|doc| doc.get_heads());

    // 1 件の追加は、その 1 件ぶんの操作に収まる
    manager
        .save_data(&doc_type, "tasks", &tasks(6, Some(2)))
        .await?;
    let (_, added_ops) = ops_since(&document.handle, &heads);
    assert!(
        added_ops < 15,
        "adding one task wrote {added_ops} operations"
    );

    // 削除も含めて、読み戻した内容は保存した内容と一致する
    manager
        .save_data(&doc_type, "tasks", &tasks(3, None))
        .await?;
    let loaded: Option<Value> = manager.load_data(&doc_type, "tasks").await?;
    assert_eq!(loaded, Some(tasks(3, None)));

    Ok(())
}
