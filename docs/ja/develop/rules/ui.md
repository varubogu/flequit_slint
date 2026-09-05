# UI 固有のコーディングルール

対象: `crates/flequit-ui`（`.slint` ファイルと ViewModel / Adapter）

## Slint 設計パターン

### 状態の置き場所

| 状態の種類 | 置き場所 |
| --- | --- |
| Rust が知る必要のない一時状態（編集中テキスト、ホバー等） | `.slint` のローカル `property` |
| 永続化・ドメイン操作・複数画面共有が必要な状態 | `global AppState` + ViewModel |
| 一覧データ | `ModelRc<T>`（実体は ViewModel が `Rc<VecModel<T>>` で保持） |
| 配色・余白・フォント | `global Theme` |
| ブレークポイント・サイズトークン | `global Layout` |
| プラットフォーム機能の可否 | `global Capabilities` |
| Rust が実装するコールバック宣言 | `global Actions` |

**原則**: Rust に持ち上げるのは必要な場合のみ。安易に `global` を増やさない。

### プロパティの向き

- 既定は `in`。`in-out` はテキスト入力の `text` など双方向が必須の場合のみ
- 派生値は `private property <T>: <式>;` で宣言的に計算する
- `out property` は親が子の内部状態を読む必要がある場合のみ

### 派生値

```slint
// ✅ 宣言的な派生（Slint が依存追跡して自動再評価）
private property <bool> is-overdue: task.due-timestamp < AppState.now-timestamp && !task.completed;

// ❌ Rust 側で計算してプロパティへ流し込む（同期漏れの温床）
```

例外: ドメインルールを含む判定（繰り返し計算、検索クエリの解釈）は ViewModel で行う。

## コンポーネント設計原則

- 機能別コンポーネントは適切なディレクトリに配置する
  （画面単位は `ui/views/`、再利用部品は `ui/components/`）
- 200 行を超える場合は機能分割を検討する
- `export component` は外部から使うものだけに付ける
- 親子間の通知は `callback` で行う。コールバック名は動詞または「動詞-目的語」
- 子コンテンツの受け渡しは `@children` を使う
- 数十件を超える可能性のあるリストは **必ず `ListView`**（仮想スクロール）を使う

### レスポンシブ

- レイアウト分岐は **画面幅** で行う。端末種別・OS で分岐しない
- ブレークポイント判定は `global Layout` を参照する
- ピクセル値をハードコードせず、`Theme` / `Layout` のトークンを使う

詳細は `docs/ja/develop/design/ui/responsive-layout.md` を参照。

### ❌ 禁止パターン: `.slint` へのロジック混入

`.slint` は「表示」のみを担当する。以下は ViewModel の責務。

- 検索クエリの解釈（`@today`、`#tag` 等）
- 繰り返しルールの日付計算
- 期限キーワードの判定
- 永続化を伴う状態変更

```slint
// ❌ 悪い例: 業務ルールを .slint に書く
private property <bool> matches: task.title.contains(query) || task.notes.contains(query);

// ✅ 良い例: ViewModel が絞り込んだ結果を Model として受け取る
in property <[TaskItem]> filtered-tasks;
```

### ❌ 禁止パターン: 1 件更新での `set_vec()`

```rust
// ❌ 全行再描画 + スクロール位置喪失
model.set_vec(all_tasks);

// ✅ 差分更新
model.set_row_data(index, updated_item);
```

## ViewModel / Adapter のルール

### Adapter は純粋関数

- ドメイン型 ↔ Slint 型の変換のみ
- I/O・副作用・グローバル参照を持たない
- 日時フォーマットはタイムゾーンを引数で受け取る

### ViewModel

- Slint の `callback` を登録する唯一の場所
- クロージャは `slint::Weak` のみをキャプチャする（強参照は循環参照になる）
- バックグラウンドから UI を更新する場合は `upgrade_in_event_loop()` を経由する
- `Rc<RefCell<T>>` の `borrow_mut()` スコープは最小化する（再入で panic するため）
- グローバル変数・`static` によるシングルトンを作らない

## レイヤーアーキテクチャ

詳細は `docs/ja/develop/design/ui/layers.md` を参照。

**重要なルール**:

- ❌ **`.slint` からドメイン型を扱うことは禁止**
- ✅ **UI 型（`.slint` の `struct`）のみを扱う**
- ❌ **ViewModel から Repository / 具象インフラへの直接アクセスは禁止**
- ✅ **必ず `flequit-core` の facade 経由でアクセス**
- ❌ **UI / core から OS API を直接呼ぶことは禁止**
- ✅ **`flequit-platform` 経由で呼ぶ**
- ❌ **`flequit-ui` に `#[cfg(target_os = ...)]` を書くことは禁止**

```rust
// ❌ NG: 具象インフラへの直接アクセス
use flequit_infrastructure_sqlite::repositories::TaskRepository;

// ✅ OK: facade 経由
use flequit_core::facades::task_facade;
```

## アクセシビリティ

対話可能な要素には以下を **必ず** 付ける。

- `accessible-role`
- `accessible-label`（プレースホルダを使う。単語連結で組み立てない）
- `accessible-action-default`（省略するとキーボード・支援技術から操作できず、
  結合テストからも到達できない）

状態を持つ要素は該当プロパティも設定する
（`accessible-checked` / `accessible-expanded` / `accessible-item-selected` など）。

## 国際化

- 全ての表示文字列は `@tr()` で記述する
- Rust 側で文言を組み立てない。識別子を渡して `.slint` でマップする
- 単語連結で文を組み立てない（語順が言語で変わるため）
- 1 語だけの文言には文脈（`"Context" =>`）を付ける

詳細は `docs/ja/develop/design/ui/i18n-system.md` を参照。

## 開発ワークフロー

詳細は `docs/ja/develop/rules/workflow.md` を参照してください。
