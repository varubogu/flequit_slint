# テスト関連のコーディングルール

## テストの種類と配置

すべて `cargo test` で実行する。テストフレームワークは Rust 標準。

| 種別 | 配置 | 実行 |
| --- | --- | --- |
| 単体テスト | 同一ソース内の `#[cfg(test)] mod tests` | `cargo test -j 4 <name>` |
| クレート内結合テスト | `crates/<crate>/tests/` | `cargo test -p <crate> -j 4` |
| クレート横断結合テスト | `tests/integration/` | `cargo test -j 4 --test <name>` |
| システムテスト | `tests/system/` | `cargo test -j 4 --test <name>` |

### UI（Slint / ViewModel）のテスト

- **`.slint` 単体のテストは行わない**。ロジックを持たないため
- ロジックの検証は ViewModel の単体テストで行う
- **Slint ウィンドウを生成しない**。`VecModel` と ViewModel の状態を直接検証する
- facade とプラットフォームはモック trait で注入する
- Adapter は純粋関数のため、入出力の対応を直接検証する
- レイアウトはブレークポイント境界のスクリーンショット比較で確認する（自動テストではない）

### 操作の結合テスト

- 配置: `crates/flequit-ui/tests/interaction.rs`
- 全ケースを **1 つの `#[test]` 関数** にまとめる（Slint のバックエンドはプロセスに 1 つ）
- ウィンドウは `show()` してから照会する（繰り返し行はレイアウト後に生成される）
- 冒頭で `slint::select_bundled_translation("en")` を呼びロケールを固定する
- 操作は `ElementHandle::find_by_accessible_label(...).invoke_accessible_default_action()`
- 新しい対話要素を追加したら **必ずここにケースを追加する**

設計上の制約と背景は `docs/ja/develop/design/testing.md` の
「操作の結合テスト（UI シェル）」を参照。

### プラットフォーム抽象化のテスト

- `flequit-platform` は trait ベースで定義し、`MockPlatform` を注入する
- ディレクトリ解決は `tempfile` を使ったテスト用実装に差し替える
- 各プラットフォーム固有実装は CI のクロスコンパイルでビルド確認する
  （実行はデスクトップと Android エミュレータ）

## テストデータ管理

- **テストデータ**: `crates/flequit-testing/` のビルダーを使用する
- **基本原則**: 1 関数 = 1 テストデータ生成

## テスト実行ルール

### 重要な制約

- **`cargo test` は必ず `-j 4`** でワーカー数を制限する
  （SQLite テスト DB の競合とメモリ枯渇を避けるため）
- **テストタイムアウト**: ファイル内テスト件数 × 1 分
- **段階的実行**: 対象を絞ったテスト → クレート単位 → 全体 の順で広げる

### テストコマンド

- `cargo test -j 4` - 全テスト実行
- `cargo test -j 4 <test_name>` - 対象テスト実行
- `cargo test -p flequit-core -j 4` - クレート単位
- `cargo test -j 4 --test <integration_test_name>` - 結合テスト単体

## テスト設計原則

### 外部ファイルを使用するテスト

- 外部ファイルを使用するテストは `.tmp/tests/` 配下に、
  テストファイルとテストケースに応じたフォルダ + 実行日時フォルダを作成して利用する

  例:
  `<project_root>/tests/integration/local_automerge_repository_test.rs` の
  `test_error_handling_and_edge_cases` を `2021/09/10 12:34:56` に実行
  ↓
  `<project_root>/.tmp/tests/cargo/integration/local_automerge_repository_test/test_error_handling_and_edge_cases/20210910_123456/`

- Automerge を利用するテストの場合、automerge ファイルに加えて、
  1 つ編集するごとに `json_history` フォルダに JSON スナップショットを出力する
- テストビルド実行時にテスト対象かに関わらず、以下の手順で SQLite のファイルを用意する
  1. `<project_root>/.tmp/tests/test_database.db` を作成しマイグレーションする
     （どのテストを実行するかに関わらず処理が "必ず" "1 度だけ" 行われる）
  2. SQLite を使うテストの場合、1 で作ったファイルを各テスト用フォルダ
     （`<project_root>/.tmp/tests/...`）にコピーして使う

  このようにすることでマイグレーション回数は最小限に抑える。

- 上記のように競合しないようテスト対象のファイルを出力するため、テスト後にクリーンアップは行わない
- **パスは `flequit-platform` のテスト用実装から取得する**。
  `~/.local/share/...` のような実パスをテストに書かない

### 非同期テスト

- `#[tokio::test]` を使用する
- タイムアウトを設定し、ハングを検出できるようにする
- 実時間のスリープに依存しない（`tokio::time::pause()` を活用）

### 国際化システムのテスト

- ViewModel は識別子しか扱わないため、翻訳のモックは不要
- 言語切替の検証は統合テストで行う
  （`select_bundled_translation()` 後に表示が変わることを確認）

### エラーハンドリングのテスト

エラーハンドリングの動作をテストする際は、テスト出力をクリーンに保つために
以下の原則に従う。

#### 原則: 想定されるエラーログを抑制する

**問題**: エラーハンドリングをテストする際、実装コードが `tracing::error!` で
エラーをログ出力することが多い。これによりテストが成功してもテスト出力に
エラーメッセージが表示され、混乱を招く。

**解決策**:

1. テスト時のログレベルを調整して想定されるエラーログを抑制する
2. エラーハンドラーが正しく呼ばれたことを検証する
3. 戻り値と状態変化（ロールバック等）を検証する

**実例**:

```rust
#[tokio::test]
async fn ネットワークエラーを適切に処理できる() {
    // Arrange: 失敗するモックを用意
    let mut facade = MockTaskFacade::new();
    facade.expect_update()
        .returning(|_, _, _| Err(ServiceError::ExternalService("network error".into())));

    let vm = TaskViewModel::new_for_test(Arc::new(facade));
    vm.insert_row(sample_task_item("t1", "元のタイトル"));

    // Act: 楽観的更新 → 永続化失敗
    vm.update_title("t1", "新しいタイトル").await;

    // Assert: ロールバックされていること
    assert_eq!(vm.row_by_id("t1").unwrap().title, "元のタイトル");
    // Assert: エラーが記録されていること
    assert!(matches!(vm.last_error(), Some(UiError::Service(_))));
}
```

**このアプローチの利点**:

- ✅ テスト成功時にテスト出力がクリーンに保たれる
- ✅ エラーハンドリングの動作とロールバックの両方を検証できる
- ✅ 想定外のエラーが発生した場合はテストが失敗する
- ✅ レビュアーがテストの意図を理解しやすい

**アンチパターン**: モック実装にデバッグ用の出力を追加しないこと。
デバッグに必要な場合は、調査後に削除する。

```rust
// ❌ 悪い例: モック内の不要なログ出力
.returning(|id| {
    if !data.contains_key(id) {
        tracing::error!("Data not found: {id}");  // これを削除！
        return None;
    }
    data.get(id).cloned()
})

// ✅ 良い例: 単に値を返すだけ
.returning(|id| data.get(id).cloned())
```

### テストの独立性

- 各テストで ViewModel / ストアを新規生成し、状態を共有しない
- グローバル変数・`static` に依存するテストを書かない
- テストの実行順序に依存しない
