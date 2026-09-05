# テスト環境

## 現在の構成

- **テストランナー**: `cargo test`（単体・結合・システムテストすべて）
- **テスト種類**: ユーティリティ関数、ドメインロジック、Repository、ViewModel、統合テスト

Slint 版は Rust 単一言語のため、Vitest / Playwright のような
別言語のテスト基盤は不要になった。

## 依存クレート

| クレート | 用途 |
| --- | --- |
| `tokio` (`macros`, `rt`) | `#[tokio::test]` による非同期テスト |
| `mockall` | trait のモック生成（facade / repository / platform） |
| `tempfile` | 一時ディレクトリ・一時ファイル |
| `rstest` | パラメタライズドテスト |
| `pretty_assertions` | 差分の読みやすい assert |
| `flequit-testing`（自作） | テストデータビルダー、共通セットアップ |

## 実行コマンド

- `cargo test -j 4` - 全テスト実行（`-j 4` 必須）
- `cargo test -j 4 <test_name>` - 対象テスト実行
- `cargo test -p flequit-core -j 4` - コアビジネスロジック層のみ
- `cargo test -p flequit-infrastructure-sqlite -j 4` - SQLite インフラ層のみ
- `cargo test -p flequit-ui -j 4` - UI クレート（ViewModel / Adapter）のみ
- `cargo test -j 4 --test <integration_test_name>` - 結合テスト単体

## テストファイル構成

```text
crates/<crate>/src/**.rs           # 単体テスト（#[cfg(test)] mod tests）
crates/<crate>/tests/              # クレート内結合テスト
tests/
├── integration/                   # クレート横断の結合テスト
└── system/                        # 実際の利用シナリオに沿ったテスト
```

## テストの書き方

### 基本

```rust
#[test]
fn 進捗率を計算できる() {
    assert_eq!(calculate_progress(3, 4), 75);
}
```

### 非同期

```rust
#[tokio::test]
async fn タスクを取得できる() {
    let repo = setup_repository().await;
    let task = repo.find_by_id(&task_id).await.unwrap();
    assert!(task.is_some());
}
```

### モック

```rust
#[tokio::test]
async fn 永続化失敗時にロールバックされる() {
    let mut facade = MockTaskFacade::new();
    facade.expect_update()
        .returning(|_, _, _| Err(ServiceError::ExternalService("boom".into())));
    // ...
}
```

## ViewModel テスト

✅ **Slint ウィンドウを生成せずに検証できる**。
ViewModel は `Rc<VecModel<T>>` と内部状態を持つだけなので、UI を起動する必要がない。

### テスト対応状況

1. **ユーティリティ関数のテスト** ✅
2. **ドメインロジックのテスト** ✅
3. **Repository / 永続化のテスト** ✅
4. **ViewModel のテスト** ✅
5. **Adapter（型変換）のテスト** ✅
6. **`.slint` のレイアウト** — 自動テスト対象外（スクリーンショット比較で確認）

### 検証対象

| 対象 | 検証内容 |
| --- | --- |
| `Model` の内容 | 行数、各行のフィールド値 |
| `Model` の更新方法 | 差分更新が使われているか（`set_vec` の乱用がないか） |
| 楽観的更新 | 即時反映されるか |
| ロールバック | 永続化失敗時に元の値へ戻るか |
| エラー状態 | `UiError` が記録されるか |

### モックの注入

`AppViewModel::new_for_test(mock_infrastructure, mock_platform)` を使い、
実 DB とプラットフォーム API に触れずに検証する。

## 操作の結合テスト（UI シェル）

ViewModel の単体テストは「ハンドラが正しく動くか」しか見ない。
**「操作がハンドラまで届くか」** は別の問題であり、実際に届いていない不具合が起きた。

`crates/flequit-ui/tests/interaction.rs` が、表示なしで UI を駆動して検証する。

### 仕組み

| 項目 | 内容 |
| --- | --- |
| バックエンド | `i-slint-backend-testing` の `init_no_event_loop()` |
| 操作の注入 | アクセシビリティツリー経由（`ElementHandle::find_by_accessible_label` → `invoke_accessible_default_action`） |
| 前提 | 対象要素に `accessible-role` と `accessible-action-default` があること |

### 制約（回避不能なので設計に織り込む）

1. **マウスイベント注入は使えない**
   `send_mouse_click` は `i-slint-backend-testing` の `internal` フィーチャ配下にあり、
   公開版はビルドできない（Slint リポジトリ内のパスを `include_dir!` している）。
   そのためアクセシビリティ経由で駆動する。
   結果として **ヒットテスト（要素の重なり）は自動検証できない**。
2. **Slint のバックエンドはプロセスに 1 つ**
   テストを並列実行するとウィンドウ生成が失敗する。
   全ケースを **1 つの `#[test]` 関数** にまとめる。
3. **`show()` が必要**
   `for` による繰り返し行はレイアウトが走るまで生成されず、
   アクセシビリティツリーに現れない。
4. **デバッグ情報が必要**
   `slint-build` の `with_debug_info(true)` を debug ビルドで有効にしている
   （`crates/flequit-ui/build.rs`）。リリースビルドでは無効。
5. **ロケールを固定する**
   翻訳は実際に効くため、システム言語によってラベルが変わる。
   テスト冒頭で `slint::select_bundled_translation("en")` を呼ぶ
   （最初のコンポーネント生成後でなければ反映されない）。

### 検証していること

- サイドバーのプロジェクト選択 / 展開 / タスクリスト選択
- 期限フィルタ
- タスク行の選択と完了トグル
- 設定ボタン

新しい操作を追加したら、このテストにもケースを追加する。

## 翻訳システムのテスト

Slint 版では ViewModel が識別子しか扱わないため、**翻訳のモックは不要**。
Svelte 版で必要だった `setTranslationService()` 相当の仕組みは存在しない。

### 検証対象

- **ViewModel**: 翻訳に依存しないため、通常どおりテストする
- **言語切替**: 統合テストで `select_bundled_translation()` 後に
  表示文字列が変化することを確認する
- **翻訳漏れ**: CI で `.pot` の再生成差分と、各 `.po` の未翻訳エントリ数を検査する

## プラットフォーム別テスト

| 対象 | 方法 |
| --- | --- |
| デスクトップ（Windows/macOS/Linux） | CI で `cargo test -j 4` を実行 |
| Android | CI でクロスコンパイル確認 + エミュレータで主要シナリオ |
| iOS | CI でクロスコンパイル確認（実行は手動） |
| プラットフォーム抽象化 | `MockPlatform` による単体テスト |

`flequit-platform` の各実装は、共通の trait に対する
**同一のテストスイート** を各プラットフォームで実行する形にする。

## レイアウト検証

自動テストの対象外。以下の手順で確認する。

1. ウィンドウ幅を 599 / 600 / 1023 / 1024px に変更する
2. 各画面のスクリーンショットを取得する
3. 前回のスクリーンショットと比較する
4. `en` / `ja` の両方で実施する

詳細は `design/ui/responsive-layout.md` の「テスト方針」を参照。

## テスト戦略

1. **単体テスト**: ユーティリティ関数、ドメインロジック、Adapter
2. **結合テスト**: Facade → Repository、Automerge と SQLite の整合性、ViewModel → Facade
3. **システムテスト**: 実際の利用シナリオ（プロジェクト作成 → タスク追加 → 完了 → 削除）
4. **レイアウト検証**: ブレークポイント境界のスクリーンショット比較（手動）

## 関連

- [テストルール](../rules/testing.md)
- [テスト要件](../requirements/testing.md)
- [ViewModel アーキテクチャ](./ui/viewmodel-architecture.md)
