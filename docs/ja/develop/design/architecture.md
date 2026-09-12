# アーキテクチャ設計書

## 1. システム概要

### 1.1 アプリケーション構成

- ネイティブアプリケーション版（本リポジトリの対象）
  - UI: Slint（`.slint` 宣言的 UI + Rust バインディング）
  - ロジック/データ: Rust クレート群（同一プロセス）
  - データベース: SQLite
  - 同期用データ表現: Automerge (CRDT)
  - 対応プラットフォーム: Windows / macOS / Linux / Android / iOS
- Web アプリケーション版
  - 現時点では実装対象外（`design/api/api.md` に将来方針のみ記載）

デスクトップとモバイルは **同一バイナリ構成・同一 UI コードベース** とし、
差異は以下の 2 点に閉じ込める。

1. **OS 連携の差異** → `flequit-platform` クレートで抽象化
2. **画面サイズの差異** → Slint のブレークポイントでレイアウト切替

### 1.2 主要コンポーネント

- UI レイヤー（`crates/flequit-ui`）
  - 画面定義: `.slint` ファイル（宣言的、`ui/views/` と `ui/components/`）
  - 状態表現: Slint の `property` / `Model`（`global singleton` で集約）
  - ViewModel: `src/viewmodels/`。Slint のプロパティ更新とコールバック処理を担当し、
    ドメイン操作は `flequit-core` の facade に委譲する
  - Adapter: `src/adapters/`。ドメインモデル ↔ Slint 構造体の相互変換のみを担当
- プラットフォーム抽象化レイヤー（`crates/flequit-platform`）
  - データ/設定ディレクトリの解決、通知、ファイル選択、外部アプリ起動
  - 実行時 Capability 判定（トレイ非対応プラットフォームでの機能無効化など）
- ビジネスロジックレイヤー（`crates/flequit-core`）
  - タスク管理 / プロジェクト管理 / 同期処理
  - facade がトランザクション境界と複数 service の協調を担当
- データアクセスレイヤー
  - Automerge をベースとしたデータ構造
  - ローカルでは SQLite を併用し、**データ検索は SQLite のみ**、
    **データ更新は SQLite → Automerge の順** で保存する
  - クラウドストレージ（Automerge）の同期も対応
  - 将来的に Git でも同期可能

## 2. アーキテクチャ

### 2.1 Slint + Rust アーキテクチャ

UI からデータアクセスまでが同一プロセス・同一言語で完結する。
IPC が存在しないため、シリアライズ境界とコマンド層が不要になる。

```text
Slint UI (.slint)
    ↕  生成された Rust バインディング (property / callback / Model)
ViewModel 層           crates/flequit-ui/src/viewmodels/
    ↓
Facade 層              crates/flequit-core/src/facades/
    ↓
Service 層             crates/flequit-core/src/services/
    ↓
Repository トレイト     crates/flequit-repository/
    ↓
永続化実装              crates/flequit-infrastructure-{sqlite,automerge}/
```

`flequit-platform` は上記の縦の流れとは独立した横断クレートで、
ViewModel 層と永続化実装層の双方から参照される。

- **UI (`.slint`)**: 表示とレイアウトのみ。ロジックを持たない
- **ViewModel**: UI 状態の保持、Slint コールバックの受け口、非同期処理の起動、
  結果の `Model` / `property` への反映
- **Facade**: アプリケーション統合ポイント。トランザクション境界を持つ
- **Service**: ドメインロジック。ストレージ非依存
- **Repository**: データアクセスの契約と具象実装

依存方向の詳細は `design/tech-stack.md` の「Rust クレート構成」を参照。

### 2.2 UI とドメインの境界

Tauri 版では IPC が UI とドメインの物理的な境界になっていたが、
Slint 版ではその強制力がない。代わりに以下のルールで境界を維持する。

- `.slint` ファイルはドメイン型を知らない。UI 用の `struct` のみを扱う
- ViewModel 以外から `flequit-core` を呼び出さない
- `flequit-ui` は `flequit-infrastructure-*` の具象クレートに直接依存しない
  （`flequit-infrastructure` の統合 Facade 経由のみ）
- UI とコアは OS API を直接呼ばない（`flequit-platform` 経由のみ）
- 境界違反は `cargo` の依存定義と CI の依存チェックで検出する

詳細は `design/ui/layers.md` を参照。

### 2.3 プラットフォーム差異の吸収

`flequit-platform` は以下を提供する。

| 分類 | 内容 |
| --- | --- |
| パス解決 | データ / 設定 / キャッシュ / ログの各ディレクトリ |
| 通知 | ローカル通知の送出（リマインダー機能で使用） |
| ファイル選択 | インポート / エクスポート / アイコン選択 |
| 外部起動 | URL・ファイルを既定アプリで開く |
| Capability | 機能の実行時可否（トレイ、OS ゴミ箱、複数ウィンドウ 等） |
| フォームファクタ | `Desktop` / `Tablet` / `Phone` の判定ヒント |

原則:

- インターフェースは全プラットフォーム共通の trait / 関数として定義する
- 未対応機能は `Err(PlatformError::Unsupported)` ではなく
  **`Capability` の問い合わせで事前に判定** し、UI から該当項目を消す
- `#[cfg(target_os = ...)]` は `flequit-platform` の内部にのみ書く。
  他クレートに条件コンパイルを漏らさない

詳細は `design/platform/platform-abstraction.md` を参照。

### 2.4 レスポンシブ設計

UI は単一コードベースで、画面幅のブレークポイントによりレイアウトを切り替える。

| ブレークポイント | 想定端末 | サイドバー | タスク一覧 / 詳細 |
| --- | --- | --- | --- |
| `>= 1024px` | デスクトップ | 常時展開 | 2 ペイン同時表示 |
| `600px 〜 1023px` | タブレット / 小窓 | アイコンのみ（折りたたみ） | 2 ペイン同時表示 |
| `< 600px` | スマートフォン | オーバーレイ（ハンバーガー） | 1 ペインずつ切替 |

- 判定は Slint 側の `property <bool>` として宣言し、`states` でレイアウトを切り替える
- ViewModel はブレークポイント判定を持たない（UI の責務）
- タッチ操作を前提に、操作対象の最小サイズとジェスチャを設計する

詳細は `design/ui/responsive-layout.md` を参照。

### 2.5 非同期処理とイベントループ

Slint のイベントループと Tokio ランタイムを共存させる。

- アプリ起動時に Tokio ランタイムを構築し、`flequit-app` が保持する
  - デスクトップ: マルチスレッドランタイム
  - モバイル: ワーカースレッド数を抑えたランタイム（電力・メモリ制約のため）
- UI コールバックは即座にリターンし、時間のかかる処理は `tokio::spawn` へ委譲する
- バックグラウンドスレッドから UI を更新する場合は
  `slint::Weak::upgrade_in_event_loop()` を必ず経由する
  （Slint のコンポーネントは `Send` ではないため、直接触ってはならない）
- UI スレッドをブロックする同期 I/O は禁止

### 2.6 アプリケーションライフサイクル

デスクトップとモバイルでライフサイクルが異なるため、明示的に扱う。

| イベント | デスクトップ | モバイル | 対応 |
| --- | --- | --- | --- |
| 起動 | プロセス開始 | `android_main` / UIKit エントリ | ブートストラップ実行 |
| バックグラウンド遷移 | ウィンドウ非表示 | OS によるサスペンド | 未保存の変更を即時フラッシュ |
| 復帰 | ウィンドウ表示 | レジューム | データ再読込・同期再開 |
| 終了 | ユーザー操作 | OS によるプロセス破棄（予告なし） | 永続化は都度完了させ、終了処理に依存しない |

モバイルでは終了通知が届かない前提とし、**「終了時に保存する」実装を禁止** する。

### 2.7 データフロー

- ローカルファーストアプローチ
  - ローカルデータの即時反映
  - バックグラウンド同期
  - オフライン対応
- SQLite データベース
  - WAL（Write-Ahead Logging）モード
  - インメモリキャッシュ
  - トランザクション最適化

楽観的更新は ViewModel 層で行う。UI プロパティを即時更新し、
永続化失敗時にスナップショットから復元する（`design/ui/viewmodel-architecture.md` 参照）。

### 2.8 初回起動時のデータブートストラップ

- 起動時は、初期化処理の完了前に最小データの存在チェックを実行する
- 各エンティティで「削除されていないデータ」が存在しない場合のみ、最小レコードを不足分だけ作成する
  - User: `Local user`
  - Account: `Local Account`（provider: `local`）
  - Project: `My Tasks`
  - TaskList: `Inbox`（削除されていないリストを持たない全プロジェクトに 1 つ）

  タスクは必ずタスクリストに属するため、リストが 1 つも無いプロジェクトには
  **タスクを作成する手段が存在しない**。新規インストール直後でも
  タスク追加に到達できるよう、既定リストの作成をブートストラップに含める。
- 初期化処理は冪等である
  - 既存のユーザーデータを上書きしない
  - 既存のアカウントデータを上書きしない
  - 既存のプロジェクトデータを上書きしない
- このブートストラップは起動時の読み取り経路（アカウント/プロジェクト読込）から実行されるため、
  ローカル DB が空でもタスク作成フローに到達できる

## 3. Web アプリケーション版（将来検討）

### 3.1 現在の位置づけ

- 実装対象外。本リポジトリにスタブも置かない
- Svelte 版に存在した `infrastructure/backends/web` 相当の実験経路は移植しない

### 3.2 将来方針

- 同期サーバ（セルフホスト可能な Automerge 同期エンドポイント）を先に定義する
- クライアント側は `flequit-repository` に同期用 Repository 実装を追加する形で対応する
- UI を Web に載せる必要が生じた場合、Slint の WebAssembly ターゲットを第一候補とする

## 4. パフォーマンス最適化

### 4.1 UI 最適化

仮想スクロール、差分更新、宣言的な派生値、遅延ロードで UI 応答性を保つ。
個別の適用方法は `design/ui/slint-patterns.md` の「パフォーマンス」、
`Model` API の使い分けは `design/ui/viewmodel-architecture.md` の
「Model の更新パターン」が正本。

### 4.2 モバイル固有の最適化

- 起動時間はモバイルでの体感品質を左右するため、初期クエリ件数に上限を設ける
- バックグラウンド遷移時は同期タスクとタイマーを停止し、電力消費を抑える
- メモリ上限が厳しいため、Automerge ドキュメントは必要なプロジェクトのみロードする

### 4.3 コア / 永続化の最適化

- **検索は SQLite のみ**、更新は SQLite → Automerge の順（§1.2 参照）。
  Automerge ドキュメントを検索経路に使わない
- N+1 を避ける。バッチ取得または `JOIN` で 1 クエリにまとめる
- 独立した I/O は `tokio::join!` / `try_join!` で並行実行する
- SQLite は WAL モードで運用する

詳細は `design/backend/rust-guidelines.md` の「パフォーマンス最適化」を参照。

## 5. セキュリティアーキテクチャ

WebView を持たないため、CSP / XSS / CSRF といった Web 由来の攻撃面は存在しない。
代わりに以下を対象とする。

- データ保護
  - ファイル暗号化（アカウント情報は必須。`design/data/data-security.md` 参照）
  - 安全な通信（同期時の TLS）
  - アクセス制御
- プラットフォーム保護
  - 配布バイナリのコード署名（デスクトップ / Android / iOS すべて）
  - モバイルはアプリサンドボックス内のみにデータを配置する
  - 依存クレートの脆弱性監査（`cargo audit`）
  - 設定/データディレクトリのパーミッション制御

## 6. ログ

`tracing` を使用し、出力先はプラットフォームごとに切り替える
（ファイル / logcat / OSLog）。設計の詳細は
`docs/ja/develop/rules/backend.md` の「ロギング」と
`design/deployment.md` の「ログ」を参照。
