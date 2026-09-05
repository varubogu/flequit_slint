# UI とコアの接続

データの保存・読み込みは `flequit-core` の facade を通じて行う。
Tauri 版に存在した「バックエンドアダプタ層（`infrastructure/backends/`）」は
Slint 版では不要になり、**ViewModel が facade を直接呼び出す**。

> レイヤー全体の関係は [`layers.md`](./layers.md) を参照。
> facade の実装の正本は `crates/flequit-core/src/facades/`。

## Tauri 版からの変更点

| Tauri 版 | Slint 版 |
| --- | --- |
| `BackendService` インターフェース + `tauri` / `web` 実装 | 不要（facade が唯一の入口） |
| `invoke('get_tasks', { projectId })` | `task_facade::list_tasks(project_id).await` |
| JSON シリアライズ / デシリアライズ | なし（Rust の値をそのまま受け渡し） |
| `Result<T, String>`（文字列化されたエラー） | `Result<T, ServiceError>`（型付きエラー） |
| `camelCase` ↔ `snake_case` のマッピング | なし（同一言語） |
| Tauri イベントによる更新通知 | Rust のチャネル / コールバック |

シリアライズ境界が消えたことで、以下が不要になった。

- コマンドモデル（`CommandModel`）とドメインモデルの変換
- Specta による型生成
- IPC 用のエラー文字列化

## アクセス経路

```text
Slint View
    ↓ callback
ViewModel                        ← ここだけが facade を呼ぶ
    ↓ async 呼び出し
flequit-core::facades
    ↓
flequit-core::services
    ↓
flequit-repository（トレイト）
    ↓
flequit-infrastructure-{sqlite,automerge}
```

アクセス制限:

- ❌ `.slint` から Rust の関数を直接呼ぶ（`Actions` の callback 経由のみ）
- ❌ ViewModel 以外から facade を呼ぶ
- ❌ ViewModel から service / repository を直接呼ぶ
- ✅ ViewModel → facade のみ

## 取り扱うエンティティ

`project`, `tasklist`, `task`, `subtask`, `tag`, `settings`, `account`, `user`

## facade の操作パターン

各エンティティの facade は以下を提供する。

| 操作 | シグネチャ（例: task） | 戻り値 |
| --- | --- | --- |
| Create | `create(project_id, input) -> Result<Task>` | 生成されたエンティティ |
| Update | `update(project_id, id, patch) -> Result<bool>` | 変更の有無 |
| Delete | `delete(project_id, id) -> Result<()>` | - |
| Get（1 件） | `get(project_id, id) -> Result<Option<Task>>` | - |
| List（複数件） | `list(project_id, conditions) -> Result<Vec<Task>>` | - |

注意:

- `project`, `tasklist`, `task`, `subtask`, `tag`: 1 件と複数件の **両方** を提供
- `settings`, `account`: 1 件のみ提供
- Create は Tauri 版の `bool` ではなく **生成されたエンティティを返す**。
  ID 採番結果を UI が即座に使えるようにするため
- 部分更新は `patch` 型で受け取る（`design/data/partial-update-implementation.md` 参照）

### 削除系 facade のトランザクション境界

削除 facade は `flequit-core` の `TransactionalDeletionPort` を呼び出す。
SQLite のトランザクション型、削除順序、Automerge のスナップショット復元は
`flequit-infrastructure` の実装内に閉じ込める。

この境界により ViewModel はほかの facade と同じく
`Result<T, ServiceError>` だけを扱い、ストレージ固有型や repository trait を参照しない。
タスク削除は `Actions.delete-task` から ViewModel を経由して接続済み。

## エラーの受け渡し

facade は `Result<T, ServiceError>` を返す。ViewModel が UI 表示用へ変換する。

```text
RepositoryError → ServiceError → （ViewModel で変換）→ i18n キー + 表示メッセージ
```

- エラー詳細の文字列化はログ用途に限定し、`flequit-ui/src/error.rs` で表示コードへ分類する
- エラー種別ごとに i18n キーへマップし、ユーザーに解決方法を提示する
- 詳細は `design/error-handling.md` を参照

## データ更新通知

外部要因（同期、他デバイスからの変更、リマインダー）による更新を UI へ伝える仕組み。
Tauri イベントの代わりに Rust のチャネルを使う。

```text
同期処理 / バックグラウンドタスク
    ↓ tokio::sync::broadcast で ChangeEvent を送出
ViewModel の購読タスク
    ↓ upgrade_in_event_loop
Slint Model / property を更新
```

`ChangeEvent` の粒度:

| バリアント | 内容 |
| --- | --- |
| `EntityCreated { kind, project_id, id }` | 単一エンティティの追加 |
| `EntityUpdated { kind, project_id, id }` | 単一エンティティの更新 |
| `EntityDeleted { kind, project_id, id }` | 単一エンティティの削除 |
| `ProjectReloaded { project_id }` | プロジェクト全体の再読込が必要 |

- ViewModel は自分が表示中のプロジェクトのイベントのみ処理する
- 自分が起こした変更（楽観的更新済み）は、イベント発行元 ID で識別して二重適用を避ける
- 大量変更時は `ProjectReloaded` にまとめ、個別イベントの洪水を避ける

## 型のルール: 親子関係

### 親 → 子へのアクセス

リスト構造・オブジェクト構造により不要。例: `Project` は `task_lists: Vec<TaskList>` を持つ。

### 子 → 親へのアクセス

**子は 1 つ上の親の ID のみを保持** する。

```text
project (project_id)
  ↓
tasklist (tasklist_id, project_id を保持)
  ↓
task (task_id, list_id を保持)
  ↓
subtask (subtask_id, task_id を保持)
```

理由: データの正規化と依存関係の明確化。

## UI 型とドメイン型

`.slint` はドメイン型を知らない。UI 用の `struct` を `.slint` 側で宣言し、
Adapter がドメイン型から変換する。

| 層 | 型 | 例 |
| --- | --- | --- |
| Slint | `.slint` の `struct` | `struct TaskItem { id: string, title: string, due-label: string, ... }` |
| Adapter | 変換関数 | `fn to_task_item(task: &Task, tz: &Tz) -> TaskItem` |
| ドメイン | `flequit-model` | `struct Task { id: TaskId, title: String, due_date: Option<DateTime<Utc>>, ... }` |

原則:

- UI 型は **表示に必要な形** に整形済みにする
  （日時は文字列、ステータスは表示名、優先度は色トークン名）
- ID は `SharedString` として保持し、コールバックで Rust へ返す際の識別子にする
- ドメイン型をそのまま `.slint` に流さない（`DateTime<Utc>` や `Option<T>` を UI に持ち込まない）

## 新しいストレージの追加

新規ストレージ（例: 同期サーバ）を追加する場合:

1. `flequit-repository` のトレイトを実装した新クレートを作る
2. `flequit-infrastructure` の統合 Facade に組み込む
3. 振り分けロジックを `flequit-infrastructure` 内に追加する

→ **ViewModel・UI は一切変更不要**。

## 関連ドキュメント

- [UI レイヤーアーキテクチャ](./layers.md)
- [ViewModel アーキテクチャ](./viewmodel-architecture.md)
- [Rust 設計ガイドライン](../backend/rust-guidelines.md)
- [部分更新システム](../data/partial-update-implementation.md)
- [全体アーキテクチャ](../architecture.md)
