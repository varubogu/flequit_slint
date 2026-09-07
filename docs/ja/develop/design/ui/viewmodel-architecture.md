# ViewModel アーキテクチャ

UI 状態の保持（Slint の `property` / `Model`）と、ドメイン操作（`flequit-core`）の
橋渡しを担う **ViewModel 層** の設計指針を定義する。

> 実装の正本は `crates/flequit-ui/src/viewmodels/` を参照。

## 目的

- Slint の宣言的 UI に必要な状態を一箇所に集約し、更新経路を一本化する
- 副作用（永続化・通知・ファイル I/O）を ViewModel に閉じ込め、View を純粋に保つ
- 初期化順序と依存関係を明確化し、循環参照を防ぐ
- テスト容易性を確保する（ViewModel は UI を起動せずに検証できる）

## レイヤー構成

```text
Slint View (.slint)
    ↕ property / callback / Model
ViewModel               crates/flequit-ui/src/viewmodels/
    ├─→ Adapter         ドメイン型 ↔ UI 型の変換（純粋関数）
    ├─→ flequit-core    facade によるドメイン操作
    └─→ flequit-platform 通知・ファイル選択
```

Svelte 版の 3 層（Stores / Services-Operations / Services-Backend）は、
Slint 版では以下に対応する。

| Svelte 版 | Slint 版 | 配置 |
| --- | --- | --- |
| Stores（状態管理） | Slint `global` の `property` / `Model` | `ui/globals/`、実体は ViewModel が保持 |
| Services - Operations（ビジネスロジック + 楽観的更新） | ViewModel | `src/viewmodels/` |
| Services - Backend（永続化） | `flequit-core` の facade | Rust コア側に吸収 |
| Services - UI（UI 特有操作） | ViewModel | `src/viewmodels/`（層を分けない） |

Svelte 版で「廃止予定」とされていた UI Services 層は、Slint 版では最初から作らない。

## ViewModel の分類

| 区分 | 役割 | 例 |
| --- | --- | --- |
| Entity ViewModel | 単一エンティティ集合の保持と CRUD | `TaskViewModel`, `ProjectViewModel`, `TagViewModel` |
| View State ViewModel | 選択・展開・フィルタ等のビュー状態 | `SelectionViewModel`, `TaskListUiViewModel` |
| App ViewModel | 全体の初期化・ルーティング・エラー集約 | `AppViewModel` |

依存ルール:

- `AppViewModel` → 各 ViewModel（生成と配線を担当）
- Entity ViewModel ↔ View State ViewModel の相互参照は禁止。
  必要な連携は `AppViewModel` が仲介するか、コールバックを注入する
- ViewModel から `.slint` の具体的な要素を触らない。`global` のプロパティ経由のみ

## 状態の置き場所

Slint では状態を `global singleton` の `property` として宣言し、
Rust 側が setter で値を注入する。

| 状態の種類 | 置き場所 | 理由 |
| --- | --- | --- |
| 一覧データ | `ModelRc<T>`（`VecModel` の実体は ViewModel が保持） | 差分通知で効率的に更新するため |
| 選択・展開・フィルタ | `global AppState` の `property` | View から直接参照するため |
| 一時的な入力状態（編集中テキスト等） | `.slint` のローカル `property` | Rust 側に持ち出す必要がないため |
| ローディング / エラー | `global AppState` の `property` | 全画面で共通表示するため |
| テーマ・言語 | `global Theme` / `global I18n` | 全体に影響するため |

**原則**: Rust が知る必要のない状態は `.slint` のローカルに置く。
Rust に持ち上げるのは、永続化・ドメイン操作・複数画面共有のいずれかが必要な場合のみ。

## Model の更新パターン

`VecModel` の実体は ViewModel が `Rc<VecModel<T>>` で保持し、
`ModelRc` を Slint へ渡す。

| 操作 | 使用する API | 用途 |
| --- | --- | --- |
| 初回ロード / フィルタ切替 | `set_vec()` | 全件差し替え |
| 1 件追加 | `push()` / `insert()` | タスク追加 |
| 1 件更新 | `set_row_data()` | タイトル変更、完了トグル |
| 1 件削除 | `remove()` | タスク削除 |

❌ 1 件の変更で `set_vec()` を呼ぶことは禁止（全行再描画とスクロール位置喪失を招く）。

## 楽観的更新パターン

すべての変更操作はこのパターンに従う。

1. 変更対象の現在値をスナップショット（`row_data()` で取得）
2. Slint の `Model` / `property` を即座に更新（楽観的更新）
3. `tokio::spawn` で facade を呼び出し永続化を試行
4. 失敗時は `upgrade_in_event_loop` でスナップショットから復元し、
   `AppState.error` にエラーを記録する

### メリット

- ユーザー体験の向上: UI が即座に反応する
- データ整合性: エラー時の自動ロールバック
- デバッグ容易性: エラーハンドリングが一箇所に集約される

### 注意点

- スナップショットは **UI 型** で取る（ドメイン型への往復変換を挟まない）
- ロールバック中に別の更新が入ることを考慮し、対象行の ID を照合してから復元する

## スレッド境界

Slint のコンポーネントと `Model` は `Send` ではない。
バックグラウンドから触ってはならない。

```text
UI スレッド                        Tokio ワーカー
─────────────────────────────────────────────────
callback 発火
  → 楽観的更新（Model 直接操作）
  → tokio::spawn ────────────────→ facade 呼び出し（async）
                                     ↓ 結果
  ← upgrade_in_event_loop ←─────────┘
  → Model / property を更新
```

規則:

- `tokio::spawn` へ渡すクロージャは `slint::Weak<AppWindow>` のみをキャプチャする
- クロージャ内で `Model` を直接触らない
- `upgrade_in_event_loop` のクロージャ内でのみ UI を更新する
- ウィンドウが既に閉じられている場合に備え、`upgrade` の失敗を握りつぶさずログに残す

## 初期化規則

1. `flequit-app` がプラットフォーム・ロガー・Tokio ランタイム・インフラを初期化する
2. `AppViewModel::new(infrastructure, platform)` で全 ViewModel を生成する
3. `AppViewModel::bind(&app_window)` で Slint のコールバックを登録し、
   初期プロパティを注入する
4. `AppViewModel::load_initial()` で初期データを非同期ロードする
5. `app_window.run()` でイベントループを開始する

テスト用には `AppViewModel::new_for_test(mock_infrastructure, mock_platform)` を用意し、
Slint ウィンドウを生成せずに検証できるようにする。

## 依存注入ガイドライン

- ViewModel はインフラ Facade とプラットフォームをコンストラクタ引数で受け取る
- グローバル変数・`static` によるシングルトンを作らない
- ViewModel 間の連携が必要な場合はコールバック（`Box<dyn Fn>`）を注入する。
  相互に `Rc` を持ち合わない（循環参照によるリークを防ぐ）
- `Rc<RefCell<T>>` を使う場合、`borrow_mut()` の生存期間を最小化する
  （Slint のコールバック内で再入すると panic するため）

## 命名・配置

| 種別 | パス | 例 |
| --- | --- | --- |
| App ViewModel | `viewmodels/app.rs` | - |
| 機能単位のロジック | `viewmodels/<機能>/` または `viewmodels/<機能>.rs` | `viewmodels/search/`、`viewmodels/reload_gate.rs` |
| 編集ダイアログの状態 | `viewmodels/<entity>_editor.rs` | `viewmodels/project_editor.rs` |
| Adapter | `adapters/<entity>.rs` | `adapters/task.rs` |

上の「ViewModel の分類」は役割の分け方であって、型やディレクトリの分け方では
ない。Entity ViewModel（`TaskViewModel` 等）と View State ViewModel は個別の型に
していない。Slint のコールバックとプロパティは `AppWindow` 1 つに対して登録する
ため、分割すると配線がエンティティ横断で散らばるだけで、状態の持ち主は増えない。
実際に切り出しているのは、**ウィンドウを作らずにテストできる純粋ロジック**
（検索、並び替え、繰り返しの次回計算、再読込の合流、設定）で、それが
`viewmodels/<機能>/` にあたる。残る Slint との配線は `viewmodels/app.rs` に集約する。

### メソッド命名

- **読み取り**: `task_by_id()`, `tasks_by_list()`
- **UI 状態のローカル更新**: `insert_row()`, `remove_row()`, `apply_row_update()`
  （明示的にローカル操作と分かる名前）
- **ユーザー操作の受け口**: `on_add_task()`, `on_toggle_status()`
  （Slint コールバックと 1 対 1 対応）
- **ドメイン操作**: `add_task()`, `update_task()`, `delete_task()`

## ベストプラクティス

### ✅ 推奨

1. View からは ViewModel のハンドラのみを呼ぶ
2. ViewModel はドメイン操作を facade に委譲する
3. Adapter は純粋関数に保つ
4. 差分通知で `Model` を更新する

### ❌ 非推奨

- `.slint` に業務ルールを書く
- ViewModel から Repository を直接呼ぶ
- UI スレッドで同期 I/O を実行する
- 1 件変更のたびに `set_vec()` する

## エラー処理

- ドメイン層のエラー（`ServiceError`）は ViewModel で UI 表示用メッセージへ変換する
- 変換は Adapter の `error.rs` に集約し、i18n キーへマップする
- `AppState.error` に積み、View 側でトースト / インライン表示を出し分ける
- panic させない。`unwrap()` は「起こり得ない」ことを証明できる箇所のみに限る

## テスト方針

- ViewModel の単体テストは Slint ウィンドウを生成せずに実行する
  （`Model` は `VecModel` を直接検証する）
- インフラ Facade とプラットフォームはモックを注入する
- 楽観的更新のロールバックは、失敗を返すモックで必ず検証する
- 各テストで ViewModel を新規生成し、状態を共有しない

## 関連ドキュメント

- [UI レイヤーアーキテクチャ](./layers.md)
- [Slint 設計パターン](./slint-patterns.md)
- [コアとの接続](./core-bridge.md)
- [レスポンシブレイアウト](./responsive-layout.md)
