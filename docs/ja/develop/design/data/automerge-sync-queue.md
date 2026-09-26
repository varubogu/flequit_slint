# Automerge 同期キュー設計書

書き込みを SQLite だけで確定させ、Automerge への反映はキューを介してバックグラウンドで行う仕組み。
Automerge の保存を待たずに操作を終えることで、UI の応答を速くする。

> 実装の正本は `crates/flequit-infrastructure/src/automerge_sync/` と
> `crates/flequit-infrastructure-sqlite/src/infrastructure/automerge_sync_queue.rs`。
> 本書は方針・状態遷移・運用ルールのみを述べる。
> テーブル定義は [`entity/automerge-sync-queue.md`](./entity/automerge-sync-queue.md)。

## 1. 背景

以前は統合リポジトリ（`flequit-infrastructure/src/unified/`）が書き込みのたびに
SQLite と Automerge の両方へ順に書き込み、両方が終わってから応答していた。
削除は SQLite のトランザクションの中で Automerge を更新し、失敗したら Automerge を
スナップショットから戻していた。

Automerge への書き込みはプロジェクト単位のドキュメント全体を読み、書き換えて保存するため遅かった
（その後、集合を Map にして差分だけを書く形に改めた。[`automerge-structure.md`](./automerge-structure.md)）。
読み取りは SQLite だけで完結しているので、Automerge が書き込みの応答時間に入っている必要はない。

## 2. 全体の流れ

```text
統合リポジトリ（書き込み）
  BEGIN
    1. キューへ INSERT（Automerge へ反映する変更。最初の文）
    2. SQLite 本体の CRUD（*_with_txn）
  COMMIT ── 失敗したら ROLLBACK（キューの行も消える）
    │ 通知
    ▼
ワーカー（バックグラウンド）
    3. キューを id 順に読み、Automerge へ適用
    4. 適用に成功したら processed に更新
    5. processed になって 30 日経った行を削除
```

- **原子性**: エンティティの変更とキューの行は同じトランザクションで確定する。
  SQLite に残った変更は必ずキューにもあり、ロールバックした変更はどちらにも残らない
- **応答**: 呼び出し側（Facade → ViewModel）はコミットの時点で戻る。Automerge の保存を待たない
- **読み取り**: 画面の読み取りはこれまでどおり SQLite だけで行う。Automerge の遅れは見えない

## 3. キューの状態

| 状態 | 意味 | 次の状態 |
| --- | --- | --- |
| `pending` | Automerge へ未反映。失敗後の再試行待ちも含む | `processed` / `failed` |
| `processed` | Automerge への適用が成功した | 30 日後に削除 |
| `failed` | 反映を諦めた。調査用に残し、自動では削除しない | （なし） |

失敗した `pending` の行には `attempts`（試行回数）、`last_error`、`next_attempt_at`（次に試す時刻）が入る。

## 4. 変更の表現

1 行が Automerge への 1 回の操作に対応する。`payload` は `AutomergeChange` の JSON。
各種類は、キュー導入前に統合リポジトリが Automerge リポジトリへ直接行っていた呼び出しと 1 対 1 に対応する。

| 対象 | 操作 |
| --- | --- |
| アカウント・ユーザー・プロジェクト | 保存 / 削除 |
| タスクリスト・タスク・サブタスク・タグ・繰り返しルール | 保存 / 削除 |
| タスク・サブタスクのタグ、担当者、繰り返し（関連） | 追加 / 解除 / 親の関連をすべて解除 / タグを指す関連をすべて解除 |
| タグブックマーク | 作成 / 更新 / 削除 |
| ゴミ箱（論理削除） | プロジェクト・タスクリスト・タスク・タグの削除と復元 |

- **反映先ドキュメント**（`document_key`）: `project:{project_id}`、`account`、`user` のいずれか。
  タグブックマークはユーザードキュメントに入るので `user`
- **互換性**: アプリの更新をまたいでキューに行が残ることがある。形式を変えるときは古い行を読めるようにする
  （フィールドの追加は既定値付き、それ以外は新しい種類を足す）

## 5. ワーカー

### 起動と停止

- `flequit-app` が起動時に開始する。最初に前回の残りを反映し、処理済みの古い行を掃除する
- コミットのたびに通知を受けて起きる。通知が無くても 60 秒ごとにキューを見る
- アプリ終了時は、残りの行を最大 3 秒まで反映してから止まる。
  間に合わなかった行は SQLite に残り、次回起動時に反映される

### 適用の順序

- 行は id 順（キューに入った順）に 1 件ずつ適用する
- 同じドキュメントの行は必ずこの順を守る。先行する行が再試行待ちの間、
  同じドキュメントの後続の行は追い越さない
- 別のドキュメントの行は止めずに進める

### 失敗と再試行

| 失敗の種類 | 扱い |
| --- | --- |
| 一時的なエラー（I/O、Automerge の読み書きなど） | `pending` のまま、1, 2, 4, … 秒（上限 5 分）の間隔で再試行する |
| 10 回失敗した | `failed` にする |
| 入力が原因で再試行しても直らないエラー（`payload` を読めない、未対応の操作、検証エラー、変換エラー） | すぐに `failed` にする |

`failed` にした行は同じドキュメントの後続を止めない。後続が同じエンティティの保存なら
（保存はそのエンティティを新しい内容に合わせる）、その時点で Automerge は SQLite に追いつく。

### 冪等性

ワーカーは各行を **少なくとも 1 回** 適用する（適用後、`processed` へ更新する前に終了すると、
次回もう一度適用する）。そのため適用は冪等にする。

- 保存はエンティティを新しい内容に合わせる（変わったフィールドだけを書く。
  [`automerge-structure.md`](./automerge-structure.md#書き込みは差分だけ)）
- 削除・関連の解除は「すでに無い」を成功として扱う
- 復元は「すでに削除済みでない」を成功として扱う

### 掃除

`processed` になって 30 日（`processed_at` 基準）経った行を削除する。
起動時と、その後 24 時間ごとに行う。`pending` と `failed` は消さない。

## 6. トランザクションとロック

- **キューの INSERT をトランザクションの最初の文にする**。SQLite は読み取りから書き込みへ
  切り替わるときに別の接続が書き込み中だと、待たずに `SQLITE_BUSY` を返すことがある。
  最初に書き込めば書き込みロックの取得をビジータイムアウト内で待つ形になり、
  ワーカーの状態更新と重なっても失敗しない
- ワーカーの SQLite 更新（`processed` への更新など）は 1 文ずつ自動コミットで行い、ロックを短く保つ
- `*_with_txn` の中で「同じトランザクションで作った行」を読む必要がある場合は、プールの別接続ではなく
  渡されたトランザクションで読む（例: タスク保存時のタグ存在確認。プロジェクトの復元でタグとタスクを
  1 トランザクションで戻すため）

## 7. Automerge を読む処理（読み取りバリア）

Automerge にしか無いデータを読む処理は、キューが未反映だと古い内容を読んでしまう。
現状これに当たるのは **ゴミ箱からの復元** だけ（削除済みのデータは SQLite から消えている）。

1. 対象プロジェクトのドキュメントの未反映の行を、その場で反映する（`flush_document`）。
   再試行待ちで残った場合はエラーにする
2. Automerge から削除済みのデータを読む
3. 「SQLite への再作成 + 復元のキュー登録」を 1 トランザクションで確定する

復元は Facade から `TransactionalRestorePort`（`flequit-core`）を通して呼ぶ。
削除の `TransactionalDeletionPort` と同じく、トランザクションとキューは `flequit-infrastructure` に閉じる。

## 8. ストレージ設定ごとの動作

| SQLite | Automerge | 書き込み | ワーカー |
| --- | --- | --- | --- |
| ○ | ○ | SQLite + キュー（本書の方式）。アプリはこの構成 | 起動する |
| ○ | × | SQLite だけ。キューには入れない | 起動しない |
| × | ○ | Automerge へ直接書き込む（キューを置く場所が無いため） | 起動しない |

## 9. 制約と既知の制限

- **Automerge のファイル書き込みの完了は確認できない**。Automerge-Repo（0.3）はドキュメントの変更を
  バックグラウンドのスレッドで保存し、完了を通知しない（失敗もログに出すだけ）。
  そのため `processed` は「Automerge ドキュメントへの適用が成功した」ことを表す。
  適用直後の数ミリ秒の間にプロセスが落ちると、その変更はファイルに残らず再送もされない。
  キュー導入前も同じ条件だった
- **Automerge は SQLite より遅れる**。ワーカーが追いつくまで Automerge は古い。
  将来の端末間同期は、送る前に対象ドキュメントを `flush_document` で反映させること
- **`failed` の行を再投入する手段は未実装**。ログと行の `last_error` で調査する
- **Automerge に対応する操作が無い書き込みはキューに入れない**。
  `SubtaskRecurrenceRepositoryTrait` の `save` / `delete_by_subtask_id` / `delete_by_recurrence_rule_id`
  はプロジェクト ID を持たず、Automerge 側が未対応のため SQLite だけに書く
  （関連の追加・解除は `ProjectRelationRepository` 側を使う）

## 10. 実装参照

| 役割 | 場所 |
| --- | --- |
| テーブルとマイグレーション | `crates/flequit-infrastructure-sqlite/src/migrator/m20260920_000003_automerge_sync_queue.rs` |
| キューの SQLite リポジトリ | `crates/flequit-infrastructure-sqlite/src/infrastructure/automerge_sync_queue.rs` |
| 変更の型 | `crates/flequit-infrastructure/src/automerge_sync/change.rs` |
| Automerge への適用 | `crates/flequit-infrastructure/src/automerge_sync/apply.rs` |
| トランザクションとキュー登録 | `crates/flequit-infrastructure/src/automerge_sync/queue.rs` |
| ワーカー・再試行・掃除 | `crates/flequit-infrastructure/src/automerge_sync/worker.rs` |
| 削除と復元 | `crates/flequit-infrastructure/src/infrastructure_repositories/transaction/` |
| 起動と停止 | `crates/flequit-app/src/lib.rs` の `run()` |
| 統合テスト | `crates/flequit-infrastructure/tests/automerge_sync_queue.rs` |

## 11. 以前の方式との違い

| 項目 | 以前 | 同期キュー |
| --- | --- | --- |
| 保存・更新 | SQLite と Automerge に順に書き込み、両方を待つ | SQLite + キューをコミットして戻る |
| 削除 | SQLite のトランザクション内で Automerge を更新。失敗時はスナップショットから復元 | SQLite の削除と論理削除のキュー登録を 1 トランザクションで確定 |
| 原子性 | SQLite と Automerge がずれる可能性があった（保存・更新） | SQLite とキューは常に一致。Automerge は結果整合 |
| Automerge の失敗 | 操作自体が失敗 | 操作は成功し、ワーカーが再試行する |
| タグブックマーク | Service が SQLite と Automerge を別々に呼ぶ | `TagBookmarkRepositoryPort` 1 つ。Automerge はキュー経由 |
| 復元 | Facade が Automerge と SQLite を直接操作 | `TransactionalRestorePort` に委譲。読み取りバリアを挟む |
