# Rust 設計ガイドライン

Flequit の Rust コード全体（UI を含む）の設計指針。
クリーンアーキテクチャ + 適切なエラーハンドリング + パフォーマンスの最適化を重視する。

> コード例の正本は `crates/...` を参照。本書はパターンの「形」と原則のみを述べる。

## アーキテクチャ構成

### クリーンアーキテクチャ（クレート分割）

```text
flequit-app (バイナリ: 初期化・DI・イベントループ)
    ↓
flequit-ui (Slint UI + ViewModel)
    ↓
flequit-infrastructure (統合インフラ Facade)
    ↓
flequit-infrastructure-sqlite / flequit-infrastructure-automerge (永続化実装)
    ↓
flequit-core (ドメインロジック: facade / service)
    ↓
flequit-repository (Repository trait)
    ↓
flequit-model
    ↓
flequit-types

flequit-platform (横断: flequit-types のみに依存)
```

### クレート間アクセス制御

| クレート | 依存可能先 |
| --- | --- |
| `flequit-app` | `flequit-ui`, `flequit-infrastructure`, `flequit-settings`, `flequit-platform` |
| `flequit-ui` | `flequit-core`, `flequit-infrastructure`, `flequit-platform`, `flequit-model`, `flequit-types` |
| `flequit-infrastructure` | `flequit-infrastructure-*`, `flequit-core`, `flequit-repository`, `flequit-model`, `flequit-types` |
| `flequit-infrastructure-*` | `flequit-repository`, `flequit-model`, `flequit-types`, `flequit-platform` |
| `flequit-core` | `flequit-repository`, `flequit-model`, `flequit-types` |
| `flequit-repository` | `flequit-model`, `flequit-types` |
| `flequit-model` | `flequit-types` |
| `flequit-platform` | `flequit-types` |

### 各クレート内部のアクセス制御

- **ViewModel**: インフラ Facade / facade 経由のみ。Repository への直接アクセス禁止
- **flequit-core / facade**: `service` のみ呼び出し可。`facade` 同士・ViewModel 直呼び・repositories 直呼び禁止
- **flequit-core / service**: repository の trait/契約参照のみ。UI やインフラ具体実装への依存禁止
- **インフラ系アダプタ (sqlite/automerge)**: 永続化実装の責務のみ。ドメインルールを実装しない
- **統合インフラ Facade**: 複数インフラ実装の合成のみ。UI 責務を持たない
- **flequit-platform**: OS 連携のみ。ドメイン知識を持たない

### 条件コンパイルの隔離

`#[cfg(target_os = ...)]` / `#[cfg(target_arch = ...)]` は
**`flequit-platform` の内部にのみ** 書く。
他クレートに漏れていないことを CI で検証する（`scripts/check-crate-deps.sh`）。

理由: プラットフォーム分岐が散在すると、対応プラットフォーム追加時の影響範囲が
追跡不能になり、ビルドできる組み合わせが把握できなくなる。

## Option 値の処理パターン

- 単一 Option: `if let Some(x) = ...` で取り出す
- 複数 Option: ネストを避けるため一時変数に格納し、最後に `match` でまとめて検証する
- 複雑な変換: `Option::and_then` / `Option::filter` / `Option::map` のチェーンを活用

実装参照:

- `crates/flequit-core/src/services/task_service.rs` - 単一 Option パターン
- `crates/flequit-core/src/services/task_assignment_service.rs` - 複数 Option パターン
- `crates/flequit-core/src/facades/task_facades.rs` - チェーン処理

## エラーハンドリング

### 階層化されたエラー型

各層に独自のエラー型を定義し、`thiserror::Error` を派生する。

| 層 | エラー型 | 役割 |
| --- | --- | --- |
| Repository | `RepositoryError` | Database (`#[from] sqlx::Error`) / Automerge / Serialization (`#[from] serde_json::Error`) / Io (`#[from] std::io::Error`) |
| Domain (service/facade) | `ServiceError` | NotFound / Validation / BusinessRule / Repository (`#[from]`) / ExternalService |
| Platform | `PlatformError` | Unsupported / PermissionDenied / Cancelled / Io |
| UI (ViewModel) | `UiError` | Service (`#[from] ServiceError`) / Platform (`#[from] PlatformError`) |

**Tauri 版との違い**: IPC がないため `CommandError` と文字列化が不要になった。
エラーは型のまま ViewModel まで届き、表示直前に i18n コードへ変換される。

実装参照: `crates/flequit-types/src/errors/`, `crates/flequit-types/src/errors/`,
`crates/flequit-platform/src/error.rs`

### コンテキスト付きエラー

長い処理チェーンでは `anyhow::Context` の `.context(...)` / `.with_context(|| ...)` で
エラーに文脈を付与する。`Result` 型の `?` で伝播させる。

ただし、**UI へ返す境界（facade の公開 API）では `anyhow` を使わない**。
型付きの `ServiceError` に落として返す。

## モジュール設計パターン

### Repository パターン

- Repository は `flequit-repository` で `#[async_trait]` トレイトとして定義
- 実装は `flequit-infrastructure-sqlite` / `flequit-infrastructure-automerge` で別々に提供
- Service は trait に依存し、実装には依存しない（依存性逆転）

実装参照:

- Trait: `crates/flequit-repository/src/repositories/task_projects/task_repository_trait.rs`
- SQLite 実装: `crates/flequit-infrastructure-sqlite/src/infrastructure/task_projects/task.rs`
- Automerge 実装: `crates/flequit-infrastructure-automerge/src/infrastructure/task_projects/task.rs`

### Service 層

- Service は依存リポジトリを `Arc<dyn Trait>` で受け取る
- 処理パターン: 1) リソース存在確認 → 2) ビジネスルール検証 → 3) 更新 → 4) 永続化 → 5) 副作用（通知等）
- 副作用の失敗は警告ログのみで継続するか、ロールバックするかをケースごとに判断

実装参照: `crates/flequit-core/src/services/task_service.rs` の `assign_task`

### Facade 層

- アプリケーション統合ポイント。トランザクション境界を持つ
- 複数 Service の協調を担う
- 公開 API は ViewModel から呼びやすい粒度にする（画面の 1 操作 = facade の 1 呼び出し）

詳細は [`transaction-management.md`](./transaction-management.md) を参照。

## 非同期処理

### ランタイム

- Tokio を使用する。ランタイムは `flequit-app` が構築し、下位層はランタイムを作らない
- デスクトップはマルチスレッド、モバイルはワーカースレッド数を抑える

### UI スレッドとの境界

- Slint のコンポーネント・`Model` は `Send` ではない。他スレッドから触らない
- バックグラウンドから UI を更新する場合は `slint::Weak::upgrade_in_event_loop()` を経由する
- `flequit-core` 以下は Slint を知らない。UI への通知はチャネル（`tokio::sync::broadcast`）で行う

詳細は `design/ui/viewmodel-architecture.md` の「スレッド境界」を参照。

### ブロッキング処理

- CPU 集約処理・同期 I/O は `tokio::task::spawn_blocking` で別スレッドへ
- I/O 並列化は `tokio::join!` / `try_join!` を使う

## パフォーマンス最適化

### データベースアクセス

- **N+1 を避ける**: 関連データはバッチ取得 / `JOIN` で 1 クエリ化
- **インデックス活用**: クエリで使うカラムに対する複合インデックスを設計時に検討
- **トランザクション**: 複数書き込みは 1 トランザクションでまとめる

詳細は [`transaction-management.md`](./transaction-management.md) を参照。

### モバイル配慮

- 起動時の初期クエリは件数上限を設ける
- Automerge ドキュメントは必要なプロジェクトのみロードし、`LowMemory` 時に解放する
- バックグラウンド遷移時に周期タスクを停止する

## 命名・スタイル

- 関数・変数・モジュール: `snake_case`
- 型・enum: `PascalCase`
- 定数: `SCREAMING_SNAKE_CASE`
- ファイル: `snake_case.rs`
- Lint: `cargo clippy` 警告ゼロを維持
- Format: `cargo fmt --all`

## ロギング

- フレームワーク: `tracing`（`tracing::info!` / `warn!` / `error!`）
- facade の公開 API には `#[tracing::instrument]` を付与する
  （`level = "info"`、`skip(...)` で大きい引数を除外、`fields(...)` で重要 ID を抽出）
- `log::` クレートは使用しない
- 出力先はプラットフォームごとに切り替える（ファイル / logcat / OSLog）。
  切替は `flequit-app` の初期化時に 1 か所で行う
- ログメッセージは英語固定（翻訳対象外）

## ドキュメンテーションコメント

- 公開 API には rustdoc を付与する
- `# Examples` を含めるとレビュー効率が高い
- 実装の意図ではなく「使い方」と「契約」を記述する

## 関連ドキュメント

- [`transaction-management.md`](./transaction-management.md) - トランザクション管理
- [`../platform/platform-abstraction.md`](../platform/platform-abstraction.md) - プラットフォーム抽象化
- [`../ui/core-bridge.md`](../ui/core-bridge.md) - UI とコアの接続
- [`../../rules/coding-standards.md`](../../rules/coding-standards.md) - コーディング規約
- [`../../rules/backend.md`](../../rules/backend.md) - バックエンドルール（アクセス制御の早見表）
