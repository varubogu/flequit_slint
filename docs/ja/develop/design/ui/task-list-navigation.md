# タスク詳細ナビゲーション設計

タスク詳細からタスクとサブタスク間をナビゲートする機能の設計。

> 実装の正本は `crates/flequit-ui/src/viewmodels/task_list_ui.rs` と
> `crates/flequit-ui/ui/views/task-list/` を参照。

## 要件

### サブタスククリック時のナビゲーション

タスク詳細（タスクビュー）でサブタスクをクリックした時:

1. タスク一覧でサブタスクを選択する
2. タスク詳細ビューにサブタスクを表示する
3. 親タスクが折りたたまれている場合は展開する
4. サブタスクがビューポート外なら自動スクロールする
5. `Compact` レイアウトでは、詳細ペインへ切り替える

### 「親タスクへ移動」ボタン

タスク詳細（サブタスクビュー）で「親タスクへ移動」をクリックした時:

1. タスク一覧で親タスクを選択する
2. タスク詳細ビューに親タスクを表示する
3. 親タスクが別タスクのサブタスクの場合、その上位タスクを展開する
4. 親タスクがビューポート外なら自動スクロールする

## アーキテクチャ

### 展開状態の管理: `TaskListUiViewModel`

タスク一覧の UI 状態（アコーディオンの展開状態を含む）を管理する ViewModel。

主な API:

| メソッド | 内容 |
| --- | --- |
| `is_expanded(task_id) -> bool` | 展開判定 |
| `toggle_expansion(task_id)` | トグル |
| `expand(task_id)` / `collapse(task_id)` | 明示的な展開/折りたたみ |
| `reset()` | 全展開状態をクリア（テスト用） |

実装上の要点:

- 展開中のタスク ID を `HashSet<TaskId>` で保持する（O(1) ルックアップ、メモリ最小）
- Slint 側へは **行データの `expanded: bool` フィールド** として反映する。
  `.slint` から `HashSet` は参照できないため、`Model` の行更新で伝える
- 展開状態の変更は `set_row_data()` による 1 行更新にとどめ、`set_vec()` を呼ばない

### Slint 側の表現

タスク行は自身の `expanded` プロパティを見てサブタスクを描画する。

```slint
export component TaskRow inherits VerticalLayout {
    in property <TaskItem> task;   // task.expanded を含む
    callback toggle-expansion(string /* task_id */);
    callback select-subtask(string /* subtask_id */);
    // ...
    if task.expanded: for sub in task.subtasks: SubTaskRow { /* ... */ }
}
```

- `.slint` は展開状態を **保持しない**。表示するだけ
- トグル操作は `Actions` のコールバックで Rust へ通知する

理由: 展開状態は将来の永続化対象であり、また詳細ペインからの操作でも
変化するため、UI ローカルに閉じると同期が破綻する。

### 自動スクロール

Slint の `ListView` に対象行を可視化させる。

- 選択変更時、対象行のインデックスを `AppState.scroll-to-index` へ設定する
- `.slint` 側は `ListView` の `viewport-y` を計算して移動する
- 既に可視範囲内にある場合はスクロールしない（不要な視点移動を避ける）

## データフロー

### サブタスククリック時

```text
タスク詳細でサブタスクをクリック
    ↓ Actions.select-subtask(subtask_id)
TaskDetailViewModel::on_select_subtask
    ↓
1. サブタスクから親タスク ID を取得
2. TaskListUiViewModel::expand(parent_task_id)
   → 親タスク行の expanded を set_row_data で更新
3. SelectionViewModel::select_subtask(subtask_id)
   → AppState.selected-subtask-id を更新
4. AppState.scroll-to-index を設定
5. Compact レイアウトなら AppState.active-pane = Detail
    ↓
タスク一覧が更新:
  - 親タスクのアコーディオンが展開
  - サブタスクがハイライト
  - タスク詳細にサブタスクが表示
```

### 親タスクへ移動時

```text
「親タスクへ移動」をクリック
    ↓ Actions.go-to-parent-task()
TaskDetailViewModel::on_go_to_parent
    ↓
1. 現サブタスクの task_id を取得
2. SelectionViewModel::select_task(parent_task_id)
3. AppState.scroll-to-index を設定
    ↓
タスク一覧が更新:
  - 親タスクがハイライト
  - タスク詳細に親タスクを表示
```

## 実装上の注意

### 借用の再入

`Rc<RefCell<T>>` で保持した状態を Slint コールバック内で `borrow_mut()` すると、
そのコールバックが別のコールバックを誘発した際に panic する。

- `borrow_mut()` のスコープを最小化する
- コールバック内で `Model` を更新する前に `borrow` を解放する

### ID の型

Slint 側は ID を `SharedString` で保持する。ViewModel 側で受け取ったら
**即座にドメインの ID 型へパースし、失敗を握りつぶさない**。

### レイアウト連動

`Compact` レイアウトでは一覧と詳細が排他表示になるため、
選択操作は必ず `AppState.active-pane` の切替とセットで行う。
`Expanded` / `Medium` では切替不要。詳細は
[`responsive-layout.md`](./responsive-layout.md) 参照。

## 依存関係

```text
TaskListUiViewModel（展開状態）
  ↑ 参照
TaskDetailViewModel（ナビゲーション操作）
  ↑ 参照
SelectionViewModel（選択状態）
```

一方向のみ。相互参照は禁止（`viewmodel-architecture.md` の依存注入ガイドライン参照）。

## テスト戦略

### 単体テスト（`TaskListUiViewModel`）

- `is_expanded` が正しい状態を返すこと
- `toggle_expansion` のトグル動作
- `expand` / `collapse` の明示的操作
- `reset` で全状態クリア

### 統合テスト

- **サブタスククリック**: 折りたたみ状態 → 詳細でクリック → 一覧で展開 + サブタスク選択
- **親タスクへ移動**: サブタスクが詳細表示 → ボタンクリック → 親タスクが選択 + 詳細表示
- **Compact 連動**: 幅 599px 相当で選択時に `active-pane` が `Detail` になること

いずれも Slint ウィンドウを生成せず、ViewModel と `VecModel` の状態で検証する。

## 移行・互換性

- **破壊的変更**: なし（純粋な機能追加）
- **後方互換性**: 完全互換

## パフォーマンス

- 展開状態は `HashSet` で O(1) ルックアップ
- 展開されたタスクのみ保存するためメモリ影響は最小
- 行更新は差分通知のみ。全件再構築しない

## 将来の拡張

- **状態永続化**: 展開状態を `user_preferences` に保存し、再起動後も保持
- **一括操作**: `expand_all()` / `collapse_all()`
- **アニメーション**: Slint の `animate` によるスムーズな展開/折りたたみ
- **キーボードナビゲーション**: 展開/折りたたみのショートカット（デスクトップ）
- **スワイプ操作**: モバイルでのスワイプによる完了/削除

## 関連

- [ViewModel アーキテクチャ](./viewmodel-architecture.md)
- [Slint 設計パターン](./slint-patterns.md)
- [レスポンシブレイアウト](./responsive-layout.md)
