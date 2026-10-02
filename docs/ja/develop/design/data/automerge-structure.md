# Automerge 構造仕様

Flequit のデータ管理は、ローカル環境での CRDT (Conflict-free Replicated Data Type) による分散同期を目的とした Automerge ベースのシステム。データは複数の Automerge ドキュメントに分散保存され、将来のクラウド同期や競合解決に対応する設計。

> 実装の正本は `crates/flequit-infrastructure-automerge/` を参照。データフロー全体は [`automerge-repo-dataflow.md`](./automerge-repo-dataflow.md) 参照。

## ドキュメント分割

データは 4 種類の Automerge ドキュメントに分割される:

| ドキュメント | ファイル | 内容 |
| --- | --- | --- |
| Settings | `settings.automerge` | 設定情報 + プロジェクト一覧 + カスタム日付/日時フォーマット + ローカル設定 |
| Account | `account.automerge` | アカウントの集合 + 現在のアカウント ID |
| User | `user.automerge` | ユーザー情報の集合 (**追加・更新のみ、削除不可**) + ユーザー設定（タグブックマーク） |
| Project | `project_{id}.automerge` | プロジェクト詳細 + タスクリスト + タスク（サブタスクを含む） + タグ + メンバー (1 プロジェクト = 1 ファイル) |

### ドキュメント間の関係

- **Settings → Project**: プロジェクト一覧から各プロジェクト詳細へナビゲーション
- **Account ↔ User**: 認証情報 (ローカル/サーバー) と公開プロフィールの関連
- **Project → User**: メンバー・担当者は `User.id` で参照
- **Project → (TaskList →) Task → Task …**: 階層的タスク管理。タスクはリストに属するか
  プロジェクト直下に置かれ、サブタスクは親を持つタスク（`parent_task_id`）として同じ `tasks` に入る

## エンティティの集合の保存形

同じ種類のエンティティの集合は、ドキュメント直下の 1 つのキーに
**「エンティティのキー → エンティティ」の Map** として置く。Automerge が推奨する形。

| ドキュメント | キー | 集合の中のキー |
| --- | --- | --- |
| Settings | `projects` | プロジェクト ID |
| Account | `accounts` | アカウント ID |
| User | `users` | ユーザー ID |
| Project | `task_lists` / `tasks` / `tags` / `recurrence_rules` / `date_conditions` / `weekday_conditions` | 各エンティティの ID |
| Project | `members` | ユーザー ID（1 ユーザーにつき 1 件） |
| Project | `task_tags` / `task_assignments` | `{task_id}:{tag_id}` / `{task_id}:{user_id}` |
| Project | `task_recurrences` | タスク ID（1 件に 1 ルール） |

タグブックマークは User ドキュメントの `user_preferences/{user_id}/tag_bookmarks/{project_id}/{tag_id}`
に置く（入れ子の Map）。プロジェクトの基本情報は Project ドキュメント直下の個別のキー
（`id`, `name`, …）に置く。

### 書き込みは差分だけ

保存するときは、既存の値と比べて **変わったフィールドだけを書く**。

- 同じ内容の保存は変更（change）を作らない
- 新しい値に無いフィールドは消す
- エンティティの中の配列（小さい値の並び）は、内容が違うときだけ丸ごと置き換える。
  同時に編集されうる集合は配列にせず、上の Map に置く

リストを丸ごと置き換える以前の形には次の問題があった。

- **同時編集が消える**: 2 つの端末が別々のタスクを編集すると、両方が `tasks` に新しいリストを置き、
  マージ後はどちらか一方のリストしか残らない。Map なら別々のエンティティ（同じエンティティの
  別々のフィールドも）への編集が両方残る
- **履歴が膨らむ**: 1 件の編集のたびに全エンティティ分の操作が積もる（タスク 17 件で 1 回約 440 操作・
  約 10KB。差分なら 1 フィールドで 1 操作・約 140 バイト）

集合全体を指定の内容にする置き換えは、スナップショットとバックアップからの復元だけで使う。
読んでから書くまでの間に追加されたエンティティまで消すため、通常の保存・削除には使わない。

### 以前のリスト形式

以前のドキュメントは集合を配列で持っている。読み取りは配列のまま受け付け、その集合に最初に
書き込むときに同じトランザクションの中で Map に変換する（キーが重複していれば後の要素が残る）。
変換を 2 つの端末で同時に行うと一方の Map だけが残るが、端末間の同期はまだ無いため問題にならない。

以前のタグブックマークの削除は値を `null` にしていた。読み取りでは `null` を無いものとして扱う。

### 以前のサブタスクの集合

以前はサブタスクを独立したエンティティとして `subtasks` / `subtask_tags` / `subtask_assignments` /
`subtask_recurrences` に置いていた。今のサブタスクは親を持つタスクで、`tasks` / `task_tags` /
`task_assignments` に入る（[`entity/projects.md`](./entity/projects.md) の Task）。

- 旧集合は、そのプロジェクトのドキュメントへ書き込む直前（同期キューの適用）と、ゴミ箱から復元する前に
  タスクへ移し、旧キーを消す。サブタスクは `parent_task_id` に元のタスク、`list_id` は null、
  旧 `completed` が立っていれば状態「完了」、優先度が無ければ 0 にする
- 移す先にすでに同じキーがあれば書かない（後から書かれた内容を古い内容で上書きしない）。
  書き終えてから旧キーを消すので、途中で止まってもやり直せる
- 旧サブタスクの繰り返しの関連は UI から書いていなかったため移さずに消す
- 一度も書き込まれないプロジェクトには旧集合が残るが、画面の読み取りは SQLite なので見え方は変わらない。
  将来 Automerge から SQLite を作り直すときは、先にこの移行を通すこと
- 実装: `crates/flequit-infrastructure-automerge/src/infrastructure/task_projects/legacy_subtasks.rs`

## データアクセスパターン

集合は `Collection`（集合のキーとエンティティのキーの求め方）を通して読み書きする。
`Document` の `load_collection` / `load_entry` / `put_entry` / `delete_entry` などが上の保存形と差分書き込みを担う。
単独の値は `Document::save_data()` / `load_data()` で読み書きする（これも差分書き込み）。

実装参照:

- 集合: `crates/flequit-infrastructure-automerge/src/infrastructure/collection/`
- 差分書き込みと JSON 変換: `crates/flequit-infrastructure-automerge/src/infrastructure/json/`
- ドキュメント管理: `crates/flequit-infrastructure-automerge/src/infrastructure/document_manager.rs`
- UI からの呼び出し口は facade。一覧は `crates/flequit-core/src/facades/mod.rs` 参照

### Tree 系 API

通常の単一エンティティ取得に加え、関連データを含む Tree 構造の取得 API も提供する。
Tauri 版ではこれらは IPC コマンドだったが、Slint 版では facade の関数となる。

- `get_user_with_assignments(user_id)`: ユーザー + 担当タスク（サブタスクを含む）
- `get_tag_with_relations(tag_id)`: タグ + 関連タスク（サブタスクを含む）
- `assign_task_to_user(task_id, user_id)`: 正規化された紐付けテーブルを使用
- `associate_recurrence_rule_to_task(task_id, recurrence_rule_id)`: 繰り返しルール関連付け

## User Document の特別な操作制約

User Document は他と異なり以下の制約がある。

### 操作制約

| 操作 | 可否 |
| --- | --- |
| 追加 | ✓ (新しいプロフィール追加は常に可能) |
| 更新 | ✓ (既存プロフィールの更新可能) |
| **削除** | **不可** (情報蓄積方式) |
| 編集権限 | 自分の `Account.user_id` にマッチするプロフィールのみ編集可能 |

### データ特性

- **公開情報**: 全プロフィールは他ユーザーから参照可能
- **情報蓄積**: プロジェクト参加者・担当者の情報を継続的に蓄積
- **プロフィール管理**: 自分と他人の公開プロフィール情報として機能

### 編集可否の判定

`current_account.user_id == target_user_id` の場合のみ編集を許可する。

実装参照: `crates/flequit-core/src/services/user_service.rs` (該当があれば)

### 更新ロジック

`update_user_profile(users, updated_user)`:

- ID 一致のユーザーがいれば既存プロフィールを置き換え
- いなければ新規追加 (削除はしない)

## 同期と競合解決

Automerge の特性により以下を実現:

- **自動競合解決**: CRDT アルゴリズムによる自動マージ
- **分散同期**: オフライン環境での操作と後の同期
- **履歴管理**: 全変更履歴の保持
- **部分同期**: ドキュメント単位での効率的な同期
