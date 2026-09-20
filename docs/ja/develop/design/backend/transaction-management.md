# トランザクション管理設計書

Flequit のデータベーストランザクション管理の設計を定義する。Facade 層でトランザクションを一元管理し、データ整合性を保証する。

> 実装の正本は `crates/...` を参照。本書は責務分離・パターン・運用ルールのみを述べる。

## 1. 設計原則

### 責務分離

| 層 | トランザクションに対する責務 |
| --- | --- |
| **Facade** | トランザクション境界の制御 (`begin` / `commit` / `rollback`)、ビジネスフロー全体の調整 |
| **Service** | ビジネスロジックの実装。トランザクションオブジェクトを引数で受け取り、Repository へ渡す |
| **Repository** | データアクセスの実装。受け取ったトランザクションオブジェクトで操作する。**自分で commit / rollback はしない** |
| **統合リポジトリ・`Transactional*Port` 実装**（`flequit-infrastructure`） | 書き込みを Automerge 同期キューへの登録と同じトランザクションにするため、自分で `begin` / `commit` / `rollback` する。`flequit-core` にはトランザクション型を見せない |

### データ整合性

- 複数の Repository 操作は **単一トランザクション内で実行**
- ACID 特性の厳格な遵守 (SQLite)
- エラー発生時の確実なロールバック

## 2. アーキテクチャ

### レイヤーフロー

```
ViewModel → Facade ──┐
                    │ begin / commit / rollback
                    ↓
                  Service ──┐ ビジネスロジック
                           ↓
                        Repository ──┐ DB アクセス
                                    ↓
                                  Database
```

### 削除処理のシーケンス例

1. ViewModel が Facade を呼ぶ (`delete_tag(repositories, ...)`)
2. Facade が `TransactionalDeletionPort::delete_tag_transactionally` に委譲する
3. インフラ層がトランザクションを開始し、最初の文で Automerge 同期キューへ
   「タグの論理削除」を登録する
4. 同じトランザクションで SQLite の関連 (tag_bookmark, task_tag, subtask_tag) とタグ本体を削除する
5. 全成功 → `commit`（コミット後にワーカーへ通知）／ 失敗 → `rollback`（キューの行も消える）

実装参照: `crates/flequit-core/src/facades/tag_facades.rs` の `delete_tag`、
`crates/flequit-infrastructure/src/infrastructure_repositories/transaction/tag.rs`

### 統合リポジトリの書き込み

保存・更新・関連の追加解除も、統合リポジトリ（`flequit-infrastructure/src/unified/`）の中で
「キュー登録 + SQLite の `*_with_txn`」を 1 トランザクションにまとめる。
Automerge へは直接書き込まない（[`../data/automerge-sync-queue.md`](../data/automerge-sync-queue.md)）。

## 3. 主要コントラクト

### `TransactionManager` トレイト

`flequit-model/src/traits/transaction.rs`

- 関連型: `type Transaction: Send + Sync`
- メソッド: `async fn begin() -> Result<Self::Transaction, RepositoryError>`, `async fn commit(txn: Self::Transaction) -> Result<...>`, `async fn rollback(txn: Self::Transaction) -> Result<...>`

### SQLite 実装

`flequit-infrastructure-sqlite/src/infrastructure/database_manager.rs`

- `DatabaseManager` が `TransactionManager` を実装
- `Transaction = sea_orm::DatabaseTransaction`
- Sea-ORM の `TransactionTrait::begin()` をラップ

### `AppContext`

`crates/flequit-app/src/lib.rs`

- `repositories: Arc<InfrastructureRepositories>` と
  `transaction_manager: Arc<dyn TransactionManager<Transaction = DatabaseTransaction>>` を保持
- Tauri 版の `AppState`（Tauri の `State` で注入）に相当するが、
  Slint 版ではフレームワークの DI 機構がないため、`flequit-app` が生成して
  ViewModel のコンストラクタへ明示的に渡す
- グローバル変数・`static` として保持しない（テストでの差し替えを可能にするため）

## 4. データベース別の扱い

| DB | トランザクション | 特徴 |
| --- | --- | --- |
| **SQLite** | 必須 | ACID 厳密。エンティティと Automerge 同期キューを同じトランザクションで確定 |
| **Automerge** | 不要 (ワーカーが 1 件ずつ適用) | CRDT による結果整合性。SQLite より遅れて追いつく |

`UnifiedRepository` は SQLite ストレージが有効なら、書き込みを
`AutomergeSyncQueue::begin`（キュー登録を最初の文にしたトランザクション）→ SQLite の `*_with_txn`
→ `QueuedTransaction::finish`（成功ならコミット、失敗ならロールバック）で行う。
Automerge 側のロールバック（以前のスナップショット復元）は不要になった。

## 5. エラーハンドリング

| 失敗箇所 | 対応 |
| --- | --- |
| トランザクション開始失敗 | Facade で format! → エラー返却 (DB 接続エラー / リソース不足) |
| ビジネスロジック失敗 | Facade で `rollback` → エラー返却 (バリデーション / 外部キー制約違反等) |
| コミット失敗 | エラー返却 (自動ロールバック) |
| ロールバック失敗 | **無視** (既にエラー状態のため)。ログには記録 |

## 6. トランザクション分離レベル

- SQLite デフォルト: **SERIALIZABLE** (最も厳格、ファントムリードなし)
- 将来的に `begin_with_isolation(level)` を追加可能 (`ReadCommitted` / `RepeatableRead` 等)

## 7. パフォーマンス

### トランザクション期間の最小化

- ✅ 検証・準備処理は **トランザクション外** で実施
- ✅ DB 操作のみをトランザクション内に
- ❌ 長時間処理 (ネットワーク呼び出し、複雑な前処理) をトランザクション内に入れない

### 読み取り専用操作

- 単純な `find_by_id` 等の読み取りは **トランザクション不要**

### バッチ操作

- 複数レコードの処理は **単一トランザクションでまとめる** (個別トランザクションを並べない)

## 8. テスト戦略

- **単体**: `mockall` で `TransactionManager` をモック化し、`begin → commit / rollback` の呼び出し回数と順序を検証
- **統合**: 実 DB を使い、削除前後のデータ整合性 (関連テーブルが正しくカスケード削除されること、ロールバック時はデータが残ること) を検証

実装参照: 各 facade の `tests` モジュール、`tests/integration/`

## 9. ベストプラクティス (要約)

### トランザクション制御

1. **Facade 層でのみ** トランザクション開始 (Service 以下では禁止 → ネストトランザクションを防ぐ)。
   例外は §1 の統合リポジトリ・`Transactional*Port` 実装（Automerge 同期キューと同じトランザクションにするため）
2. 明示的な commit / rollback (自動 commit に依存しない)
3. トランザクション期間を最小化
4. 読み取り専用操作にトランザクションを使わない

### コードレビューチェックリスト

- [ ] トランザクション開始は Facade 層のみか?（統合リポジトリ・`Transactional*Port` 実装を除く）
- [ ] Automerge へ直接書き込まず、同じトランザクションで同期キューへ登録しているか?
- [ ] 成功時にコミット、失敗時にロールバックしているか?
- [ ] トランザクション期間は最小化されているか?
- [ ] 読み取り専用操作に不要なトランザクションを使っていないか?
- [ ] エラーハンドリング (begin / commit / rollback の各失敗) は適切か?
- [ ] テストケース (成功・失敗・ロールバック検証) は十分か?

## 10. 実装状況サマリ

| Phase | 内容 | 状態 |
| --- | --- | --- |
| 1 | 基盤 (`TransactionManager` トレイト + SQLite 実装 + `InfrastructureRepositories`) | ✅ |
| 2 | パイロット実装: タグ削除 (Repository + Facade) | ✅ |
| 3 | 削除処理拡張: タスク削除 (cascade)、プロジェクト削除 (包括的 cascade) | ✅ |
| 4 | Repository 層クリーンアップ (旧 `delete_with_relations()` 廃止、内部トランザクション処理を Repository から除去) | ✅ |
| 5 | Automerge 同期キュー: 削除・復元・保存・更新・関連操作を「SQLite + キュー登録」の 1 トランザクションに統一し、Automerge への反映をバックグラウンド化 | ✅ |
| - | 複数エンティティにまたがる作成・更新（例: タスクとタグ付けの同時作成）を 1 トランザクションにまとめる | 予定 |

実装ファイル参照:

- TransactionManager: `crates/flequit-model/src/traits/transaction.rs`
- SQLite 実装: `crates/flequit-infrastructure-sqlite/src/infrastructure/database_manager.rs`
- パイロット (タグ削除): `crates/flequit-core/src/facades/tag_facades.rs`
- カスケード削除 (タスク): `crates/flequit-core/src/facades/task_facades.rs`
- カスケード削除 (プロジェクト): `crates/flequit-core/src/facades/project_facades.rs`

## 11. 新エンティティへの実装ガイドライン

1. SQLite Repository に `_with_txn` メソッドを追加 (引数に `&sea_orm::DatabaseTransaction` を受け取る)。
   同じトランザクションで作った行を読む必要があるときは、プールではなくそのトランザクションで読む
2. 統合リポジトリの書き込みで `AutomergeSyncQueue::begin(変更) → *_with_txn → finish` を使う
3. Automerge へ反映する操作を `AutomergeChange` に足し、`AutomergeSyncTargets::apply` で適用する
   （適用は冪等にする。[`../data/automerge-sync-queue.md`](../data/automerge-sync-queue.md) §5）

## 12. 参考

- Sea-ORM Transaction: <https://www.sea-ql.org/SeaORM/docs/advanced-query/transaction/>
- SQLite Transaction: <https://www.sqlite.org/lang_transaction.html>
