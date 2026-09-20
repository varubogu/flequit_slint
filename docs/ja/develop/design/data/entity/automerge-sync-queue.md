# Automerge 同期キューのエンティティ定義

ローカルの SQLite に置く、Automerge へ反映する変更のキュー。ドメインのエンティティではなく、
インフラ層の内部で使う。仕組みは [`../automerge-sync-queue.md`](../automerge-sync-queue.md) を参照。
共通フォーマットは `_template.md` を参照。

## AutomergeSyncQueue (Automerge 同期キュー) — automerge_sync_queue

**役割**: エンティティの変更と同じトランザクションで記録した「Automerge への 1 回の操作」。
バックグラウンドのワーカーが id 順に Automerge へ適用する。

### フィールド

| 論理名 | 物理名 | 型 (Rust) | 制約 | デフォルト | 外部キー | 説明 |
| --- | --- | --- | --- | --- | --- | --- |
| 連番 | id | i64 | PK, NN | 自動採番 | - | 適用順。削除後も再利用しない（AUTOINCREMENT） |
| 反映先ドキュメント | document_key | String | NN | - | - | `project:{project_id}` / `account` / `user` |
| 変更の種類 | change_kind | String | NN | - | - | `task.save` など。ログと調査用 |
| 変更内容 | payload | String | NN | - | - | `AutomergeChange` の JSON |
| 状態 | status | String | NN | "pending" | - | `pending` / `processed` / `failed` |
| 試行回数 | attempts | i32 | NN | 0 | - | 適用を試みた回数 |
| 最後のエラー | last_error | Option\<String\> | - | NULL | - | 失敗したときのエラー |
| 次に試す時刻 | next_attempt_at | Option\<DateTime\<Utc\>\> | - | NULL | - | 失敗後のバックオフ。この時刻まで再試行しない |
| 登録日時 | created_at | DateTime\<Utc\> | NN | - | - | キューへ入れた時刻 |
| 反映日時 | processed_at | Option\<DateTime\<Utc\>\> | - | NULL | - | Automerge へ反映した時刻。掃除の基準 |

### 制約

- PRIMARY KEY: `id`（AUTOINCREMENT）
- CHECK: `status IN ('pending', 'processed', 'failed')`
- NOT NULL: `id`, `document_key`, `change_kind`, `payload`, `status`, `attempts`, `created_at`

### インデックス対象カラム

`(status, id)`（未処理の行を順に読む）、`(status, processed_at)`（処理済みの古い行を消す）
（SQL 文は `crates/flequit-infrastructure-sqlite/src/migrator/m20260920_000003_automerge_sync_queue.rs` を正本とする）

### 関連

- なし（外部キーを持たない。削除済みエンティティへの変更も残す必要があるため）

### 補足

- `processed` の行は `processed_at` から 30 日で削除する。`pending` と `failed` は自動では消さない
- Rust モデル: `crates/flequit-infrastructure-sqlite/src/models/automerge_sync_queue.rs`
