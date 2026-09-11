# コーディング規約 (全体)

Flequit プロジェクトで統一されたコードスタイルと品質を保つための規約。
Slint 版は Rust 単一言語のため、言語横断の変換規約は不要になった。
本書は Rust と `.slint` の両方に共通する事項を扱う。

| 領域 | 詳細ドキュメント |
| --- | --- |
| Slint / UI 設計 | `docs/ja/develop/design/ui/slint-patterns.md`, `docs/ja/develop/rules/ui.md` |
| レイヤー / ViewModel | `docs/ja/develop/design/ui/layers.md`, `viewmodel-architecture.md` |
| レスポンシブ | `docs/ja/develop/design/ui/responsive-layout.md` |
| Rust 設計 / クレート構成 | `docs/ja/develop/design/backend/rust-guidelines.md`, `docs/ja/develop/rules/backend.md` |
| プラットフォーム抽象化 | `docs/ja/develop/design/platform/platform-abstraction.md` |
| UI ↔ コア接続 | `docs/ja/develop/design/ui/core-bridge.md` |
| テスト | `docs/ja/develop/rules/testing.md` |
| i18n | `docs/ja/develop/design/ui/i18n-system.md` |
| ドキュメント | `docs/ja/develop/rules/documentation.md` |

---

## ファイル構成

### 単一責任原則

- 1 ファイル 1 機能
- ファイル名から機能が推測できる命名
- **View と ViewModel の分離**: `.slint` は表示のみ。
  状態遷移・副作用・ドメイン呼び出しは `crates/flequit-ui/src/viewmodels/` で実装する

### ファイルサイズ

- **200 行超過**: 必須分割（テスト除く）
- **100 行超過**: 分割を検討
- **例外**: 設定ファイル・データ定義・マイグレーション
- 分割方針は `docs/ja/develop/design/ui/slint-patterns.md` の「ファイル分割指針」を参照

---

## 命名規則

### ファイル・ディレクトリ

| 対象 | 規則 | 例 |
| --- | --- | --- |
| `.slint` ファイル | ケバブケース | `task-item.slint` |
| Rust ファイル | スネークケース | `task_viewmodel.rs` |
| ディレクトリ | ケバブケース（`ui/` 配下）/ スネークケース（`src/` 配下） | `ui/task-list/`, `src/viewmodels/` |
| ドキュメント | ケバブケース | `responsive-layout.md` |

例外: テーブル名等、実物名に合わせる場合。

### 識別子

| 対象 | 規則 | 例 |
| --- | --- | --- |
| Rust: 変数・関数・モジュール | `snake_case` | `get_task_by_id` |
| Rust: 型・トレイト・enum | `PascalCase` | `TaskViewModel` |
| Rust: 定数 | `SCREAMING_SNAKE_CASE` | `MAX_TASK_COUNT` |
| Slint: component / struct / enum | `PascalCase` | `TaskItem` |
| Slint: property / callback | `kebab-case` | `is-selected`, `toggle-completed` |
| Slint: global singleton | `PascalCase` | `AppState`, `Theme` |

Slint の `kebab-case` は Rust 側の生成 API で自動的に `snake_case` になる
（`is-selected` → `set_is_selected()` / `get_is_selected()`）。

```rust
let user_name = "john";
const USER_ROLE_ADMIN: &str = "admin";
fn get_user_by_id(id: &UserId) -> Option<User> { todo!() }
struct TaskManager;
```

```slint
export component TaskRow inherits Rectangle {
    in property <bool> is-selected;
    callback toggle-completed();
}
```

---

## 型定義

### 厳密な型指定

- **ID は専用型を使う**。`String` を ID として引き回さない
  （`TaskId`, `ProjectId` 等。取り違えをコンパイル時に防ぐ）
- ドメイン値は enum で制約する（`TaskStatus::Todo` 等）。文字列で表現しない
- `Option<T>` と「空文字」を混同しない

```rust
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub status: TaskStatus,
    pub assignee: Option<UserId>,
}
```

### Slint 側の型

Slint の `struct` は `Option` を表現できない。変換時に必ず解決する。

```slint
struct TaskItem {
    id: string,
    title: string,
    due-label: string,     // 未設定は空文字
    has-due: bool,         // 未設定かどうかを明示
    status: TaskStatus,
}
```

変換規則の詳細は `docs/ja/develop/design/data/data-model.md` を参照。

---

## エラーハンドリング

### `Result<T, E>` + `thiserror`

階層化されたエラー型を使用する。詳細は
`docs/ja/develop/design/backend/rust-guidelines.md` の「エラーハンドリング」を参照。

```rust
pub async fn update_task_status(id: &TaskId, new_status: TaskStatus) -> Result<Task, ServiceError> {
    let mut task = repository.find_by_id(id).await?
        .ok_or(ServiceError::NotFound { id: *id })?;
    if !task.status.can_transition_to(new_status) {
        return Err(ServiceError::BusinessRule(
            format!("invalid transition: {:?} -> {:?}", task.status, new_status)
        ));
    }
    task.status = new_status;
    repository.save(&task).await?;
    Ok(task)
}
```

### 原則

- エラーを握りつぶさない（`let _ = ...` で捨てない）
- `unwrap()` は「起こり得ない」ことを証明できる箇所のみ
- 公開 API では `anyhow` ではなく型付きエラーを返す
- ユーザー向け文言への変換は ViewModel の表示直前でのみ行う

---

## UI とコアの接続 (要約)

詳細は `docs/ja/develop/design/ui/core-bridge.md` を参照。

- **`.slint`**: UI 型のみを扱う。ドメイン型を知らない
- **Adapter**: ドメイン型 ↔ UI 型の純粋な変換
- **ViewModel**: facade を呼び出す唯一の層。楽観的更新とロールバックを担当
- **戻り値**: Create は生成エンティティ、Update は変更有無の `bool`、
  取得は `Option<T>` / `Vec<T>`
- **スレッド**: バックグラウンドからの UI 更新は `upgrade_in_event_loop()` 経由のみ

---

## Import 順序 (Rust)

1. 標準ライブラリ（`std`, `core`）
2. 外部クレート（`slint`, `tokio`, `chrono` 等）
3. ワークスペース内クレート（`flequit_core`, `flequit_model` 等）
4. 同一クレート内（`crate::`, `super::`, `self::`）

各グループ間は空行で区切る。`cargo fmt` の設定に従う。

### `.slint` の import

- 相対パスで記述する（`import { TaskRow } from "../components/task-row.slint";`）
- 標準ウィジェットは `import { Button } from "std-widgets.slint";`
- `globals/` は各ファイルから直接 import する

### 公開範囲

- Rust: 必要最小限の `pub`。クレート内共有は `pub(crate)`
- Slint: 外部から使うものだけに `export` を付ける

---

## 関数・メソッド設計

### 純粋関数を推奨

- 計算と副作用を分離する
- 関数名から副作用が推測できない場合はリファクタ対象
- Adapter 層は必ず純粋関数にする

```rust
fn calculate_progress(completed: usize, total: usize) -> u8 {
    if total == 0 { 0 } else { ((completed * 100) / total) as u8 }
}
```

### 派生値は宣言的に

Slint のプロパティバインディングは依存追跡付きで自動再評価される。
Rust 側で計算してプロパティへ流し込む形は、同期漏れの原因になるため避ける。

```slint
private property <bool> is-form-valid: title-input.text != "";
```

詳細は `docs/ja/develop/design/ui/slint-patterns.md` を参照。

---

## ドキュメンテーションコメント

- 公開 API には rustdoc を付与する
- `# Examples` を含めるとレビュー効率が高い
- 実装の意図ではなく「使い方」と「契約」を記述する
- `.slint` の `export component` には、用途と主要 property をコメントで記述する

---

## パフォーマンスの基本

UI 側（`ListView` / 差分更新 / 派生値のバインディング）は
[`ui.md`](./ui.md)、コアとモバイルは
[`../design/architecture.md`](../design/architecture.md) の「パフォーマンス最適化」が正本。
