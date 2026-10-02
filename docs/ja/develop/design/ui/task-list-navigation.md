# タスク詳細ナビゲーション設計

タスク詳細からタスクとサブタスク間をナビゲートする機能の設計。
サブタスクは「親を持つタスク」で、深さに制限は無い（[メイン画面](./page/main/main.md) の「データ構造について」）。

> 実装の正本は `crates/flequit-ui/src/viewmodels/task_list_ui.rs`、
> `crates/flequit-ui/src/viewmodels/app.rs`（`open_task`）と
> `crates/flequit-ui/ui/views/task-list/` を参照。

## 要件

### サブタスクを開く

タスク詳細の <サブタスク> の行を押した時（どの深さのタスクでも同じ）:

1. タスク詳細にそのサブタスクを表示する（タスクと同じ `TaskDetailView`）
2. 上位のタスクの行が閉じていれば開き、一覧にサブタスクの行を表示する
3. 一覧でサブタスクの行を選択し、ビューポート外なら自動スクロールする
4. 検索で一覧に出ないサブタスクでも、タスク詳細には表示する
5. `Compact` レイアウトでは、詳細ペインへ切り替える

### 上位のタスクへ移動

サブタスクのタスク詳細の先頭には、最上位から親までのタスクをパンくずとして並べる。
押したタスクを、上と同じ手順で開く。

## アーキテクチャ

### 展開状態の管理: `TaskListUiViewModel`

タスク一覧の UI 状態（行の開閉）を管理する ViewModel。どの深さのタスクも同じように開閉する。

主な API:

| メソッド | 内容 |
| --- | --- |
| `is_expanded(task_id) -> bool` | 展開判定 |
| `toggle(task_id)` | トグル |
| `expand(task_id)` / `collapse(task_id)` | 明示的な展開/折りたたみ |
| `reset()` | 全展開状態をクリア（テスト用） |

実装上の要点:

- 展開中のタスク ID を `HashSet` で保持する（O(1) ルックアップ、メモリ最小）
- Slint 側へは **行データの `expanded: bool` フィールド** として反映する。
  `.slint` から `HashSet` は参照できないため、行の作り直しで伝える
- サブタスクは独立した行なので、開閉すると行が増減する。開閉のたびに一覧のモデルを作り直す

### Slint 側の表現

一覧は「表示する全階層の行」を表示順に並べた平坦な列で、サブタスクの行は親の行の直後に
`depth` に応じて字下げして並ぶ。Slint の struct は自分自身を入れ子にできないため、
階層は `depth` と `parent-id` で表す。

```slint
for task in AppState.tasks: TaskRow {
    task: task;   // task.depth / task.expanded / task.subtask-count を含む
    toggle-expansion => { Actions.toggle-task-expansion(task.id); }
}
```

- `.slint` は展開状態を **保持しない**。表示するだけ
- トグル操作は `Actions` のコールバックで Rust へ通知する
- 詳細ペインの <サブタスク> は `TaskItem.subtasks`（直下の子の要約）を並べる。
  パンくずは `TaskItem.ancestors`

理由: 展開状態は将来の永続化対象であり、また詳細ペインからの操作でも
変化するため、UI ローカルに閉じると同期が破綻する。

### 一覧に行が無いタスク

閉じた行の下や、検索で外れたタスクにも詳細ペインから移れる。

- 選択中のタスクに行が無いときは、キャッシュした木から `TaskItem` を作って `AppState.selected-task` に置く
- 編集の楽観的更新は、行があれば行を、無ければ `selected-task` を書き換える。
  どちらの場合も、親の行と親の詳細の <サブタスク> の要約（名前・完了・期限・完了数）を追従させる
- 再読み込み後も、選択中のタスクが木に残っていれば詳細ペインに表示し続ける

### 自動スクロール

Slint の `ListView` に対象行を可視化させる。

- 選択変更時、対象行のインデックスを `AppState.scroll-to-index` へ設定する
- `.slint` 側は `ListView` の `viewport-y` を計算して移動する
- 既に可視範囲内にある場合はスクロールしない（不要な視点移動を避ける）

## データフロー

### サブタスク・上位のタスクを開く

```text
<サブタスク> の行、またはパンくずを押す
    ↓ Actions.select-task(task_id)
open_task
    ↓
1. キャッシュした木から上位のタスクを求める
2. 閉じている上位のタスクを TaskListUiViewModel::expand
   → 1 つでも開いたら一覧のモデルを作り直す
3. 行があれば選択して AppState.scroll-to-index を設定
   行が無ければ木から TaskItem を作って AppState.selected-task へ
4. Compact レイアウトなら AppState.active-pane = Detail
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
- `toggle` のトグル動作
- `expand` / `collapse` の明示的操作
- `reset` で全状態クリア

### 統合テスト

- **サブタスクを開く**: 折りたたみ状態 → 詳細でクリック → 一覧で展開 + サブタスク選択
- **上位のタスクへ移動**: サブタスクが詳細表示 → パンくずをクリック → そのタスクが選択 + 詳細表示
- **一覧に行が無いタスク**: 検索で外れたサブタスクも木から詳細に表示できること
- **Compact 連動**: 幅 599px 相当で選択時に `active-pane` が `Detail` になること

いずれも Slint ウィンドウを生成せず、ViewModel と `VecModel` の状態で検証する。

## 移行・互換性

- **破壊的変更**: なし（純粋な機能追加）
- **後方互換性**: 完全互換

## パフォーマンス

- 展開状態は `HashSet` で O(1) ルックアップ
- 展開されたタスクのみ保存するためメモリ影響は最小
- 値の編集は行の差分更新のみ。開閉は行が増減するため一覧を作り直す

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
