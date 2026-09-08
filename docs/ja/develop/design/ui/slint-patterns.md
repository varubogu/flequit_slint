# Slint 設計パターン

Flequit で採用する Slint 中心の UI 設計パターン。

> 実装の正本は `crates/flequit-ui/ui/` および `crates/flequit-ui/src/` を参照。
> 本書は原則と適用範囲のみを述べる。

## ファイル分割指針

| 規模 | 方針 |
| --- | --- |
| シンプル（〜100 行） | 単一 component。ローカル `property` で完結させる |
| 中規模（100〜200 行） | 内部 component に分割し、同一ファイル内に置く（`export` しない） |
| 大規模（200 行超） | ファイルを分割（header / content / footer 等）。分割後も各 200 行以内を目安 |

- 1 ファイル 1 責務。`views/` は画面単位、`components/` は再利用単位
- `export component` は外部から使うものだけに付ける

## global singleton の使い分け

Slint の `global` はアプリ全体で共有される単一インスタンス。用途を明確に分ける。

| global | 責務 | 更新元 |
| --- | --- | --- |
| `Theme` | 配色・余白・フォント・角丸のトークン | Rust（テーマ切替時） |
| `Layout` | ブレークポイントとサイズトークン | ルート `Window`（幅の変化） |
| `AppState` | 選択・展開・フィルタ・ローディング・エラー | Rust（ViewModel） |
| `Capabilities` | プラットフォーム機能の可否 | Rust（起動時に 1 回） |
| `Actions` | Rust 側が実装するコールバックの宣言 | Rust（起動時に登録） |

原則:

- **データの流れを一方向に保つ**。`Actions` で Rust へ通知し、`AppState` で Rust から受け取る
- `.slint` から `AppState` を直接書き換えるのは、**Rust が知る必要のない一時状態のみ**
- 永続化やドメイン操作を伴う変更は必ず `Actions` のコールバック経由にする

## プロパティの向き

| 宣言 | 意味 | 使いどころ |
| --- | --- | --- |
| `in property` | 親（または Rust）から注入される | 表示データ |
| `out property` | 子から親へ公開する | 内部状態の読み取り |
| `in-out property` | 双方向 | テキスト入力の `text` 等に限定する |
| `private property` | component 内部専用 | 派生値・内部フラグ |

- 既定は `in` を選ぶ。`in-out` は双方向バインディングが必要な場合のみ
- 派生値は `private property <T>: <式>;` で宣言し、宣言的に計算する

## 派生値は宣言的に書く

Slint のプロパティバインディングは依存追跡付きで自動再評価される。
Svelte の `$derived` に相当する。

```slint
// ✅ 宣言的な派生
private property <bool> is-overdue: task.due-date < AppState.now && !task.completed;

// ❌ Rust 側で計算してプロパティに流し込む（同期漏れの温床）
```

例外: ドメインルールを含む判定（繰り返し計算、検索条件の解釈）は
Rust 側の ViewModel で行い、結果だけを渡す。

## コンポーネント設計

### プロパティとコールバック

```slint
export component TaskRow inherits Rectangle {
    in property <TaskItem> task;
    in property <bool> selected;
    callback clicked();
    callback toggle-completed();
    // ...
}
```

- 親子間の通知は `callback` で行う
- コールバック名は動詞または「動詞-目的語」で命名する（`clicked`, `toggle-completed`）
- 引数は最小限にする。ID だけで足りる場合は構造体を渡さない

### 子コンテンツの受け渡し

`@children` を使う。Svelte の `Snippet` に相当する。
Modal やレイアウト系コンポーネントで活用する。

### リスト表示

```slint
// 少数の固定リスト
for item in model: TaskRow { task: item; }

// 大量データ（仮想スクロール）
ListView {
    for item in model: TaskRow { task: item; }
}
```

- 数十件を超える可能性のあるリストは **必ず `ListView`** を使う
- `for` に渡す `model` は Rust 側の `VecModel` を `ModelRc` にしたもの

## Rust バインディングのパターン

### コールバック登録

```rust
let weak = app.as_weak();
app.global::<Actions>().on_add_task(move |title| {
    let Some(app) = weak.upgrade() else { return };
    // 楽観的更新 → tokio::spawn → upgrade_in_event_loop
});
```

- クロージャは `Weak` のみをキャプチャする（強参照は循環参照になる）
- クロージャ内で即座に重い処理をしない

### バックグラウンドからの更新

```rust
let weak = app.as_weak();
tokio::spawn(async move {
    let result = facade.add_task(input).await;
    let _ = weak.upgrade_in_event_loop(move |app| {
        // ここでのみ UI を触る
    });
});
```

詳細は [`viewmodel-architecture.md`](./viewmodel-architecture.md) の「スレッド境界」を参照。

## アクセシビリティ

対話可能な要素には **`accessible-role` と `accessible-action-default` の両方** を付ける。

```slint
Rectangle {
    // ...
    TouchArea { clicked => { Actions.select-project(project.id); } }
    accessible-role: button;
    accessible-label: project.name;
    accessible-action-default => { Actions.select-project(project.id); }
}
```

理由:

- `accessible-role` だけではスクリーンリーダーやキーボードから **操作できない**。
  「読めるが押せない」要素になる（`requirements/accessibility.md`）
- `accessible-action-default` は結合テストの唯一の入口でもある。
  Slint のマウスイベント注入 API (`send_mouse_click`) は
  `i-slint-backend-testing` の `internal` フィーチャ配下にあり、
  crates.io 公開版ではビルドできないため使えない
  （Slint リポジトリ内のパスを埋め込んでいる）

`accessible-label` を単語連結で組み立てないこと。プレースホルダを使う。

```slint
// ❌ 語順が言語で変わるため壊れる
accessible-label: project.name + " " + @tr("Sidebar" => "expand");
// ✅
accessible-label: @tr("Sidebar" => "Expand {0}", project.name);
```

### フォーカス表示

`TouchArea` だけの要素はキーボードで到達できない。押せる要素には `FocusScope`
を持たせ、`has-focus` の間だけ `FocusRing`（`components/focus-ring.slint`）を
描画する。共通ボタン（`IconButton` / `DialogButton` / `RowButton` /
`ChoiceButton` / `ColorPicker`）は実装済みなので、新規ボタンはこれらを使う。

```slint
key-focus := FocusScope {
    key-pressed(event) => {
        if (event.text == " " || event.text == "\n") {
            root.clicked();
            return accept;
        }
        reject
    }
}

if key-focus.has-focus: FocusRing { radius: root.border-radius; }

touch := TouchArea {
    // クリックでもフォーカスを移す。次の Tab がその位置から続く
    clicked => { key-focus.focus(); root.clicked(); }
}
```

### フォーカストラップ

Slint の Tab 送りはウィンドウ全体を巡回するため、モーダルを開いていても
背後のサイドバーやタスク一覧へフォーカスが抜ける。ダイアログのカードの
**最初と最後の子** に `FocusSentinel`（`components/focus-sentinel.slint`）を置き、
互いを `focus()` して端で折り返す。

```slint
head := FocusSentinel { wrapped => { tail.focus(); } }
// ... ダイアログの中身 ...
tail := FocusSentinel { wrapped => { head.focus(); } }
```

あわせて、ダイアログを開いた時点で **内部の要素にフォーカスを置く**
（対象要素の `init => { self.focus(); }`）。フォーカスが外にあるままでは
トラップは働かない。`if` で入れ替わる状態（削除確認など）は、切り替え先の
要素にも `init` フォーカスを持たせる。フォーカスを持っていた要素が消えると、
フォーカスは行き先を失ってダイアログの外へ出る。

`crates/flequit-ui/tests/interaction.rs` の
`a_modal_keeps_keyboard_focus_inside_itself` が全ダイアログを検証している。

## 全画面オーバーレイの注意

`AppState.loading` のような全画面 `TouchArea` は **すべての入力を飲み込む**。
フラグが下りないまま残ると、UI は正常に見えるのに一切反応しなくなる。

- 非同期処理の完了経路が失敗しうる場合、`upgrade_in_event_loop` の
  `Err` を握りつぶさずログに残す
- オーバーレイを追加したら、それを解除する経路が必ず存在することを確認する

## PopupWindow の制約

`PopupWindow` は独立したウィンドウとして開くため、次の制約がある。

- 外側のコンポーネントからは `show()` / `close()` しか呼べない。
  独自の `public function` の呼び出しやプロパティ代入は
  `Cannot access property or callback ... inside of a Window` になる
- 中身は親ウィンドウのアクセシビリティツリーに現れないため、
  `ElementHandle::find_by_accessible_label` から到達できずテストできない

そのため**選択肢リストはポップアップにしない**。`SearchComboBox` /
`SelectField` のようにその場で下方向へ展開する。

月グリッドのように大きく、フォーム全体を押し下げてしまうものだけ
`PopupWindow`（`components/calendar-popup.slint`）を使う。
「開くたびに初期化したい」状態は呼び出し側で加算するトークンを
`in property` で受け、`changed` ハンドラでリセットする。

`std-widgets` の `DatePickerPopup` は月送りボタンが内部で 48px の
最小高さを持ち行の高さまで伸びるため使わない。

## アンチパターン

- ❌ **`.slint` に業務ルールを書く**: 検索条件の解釈、繰り返しルールの計算などは ViewModel へ
- ❌ **Rust 側での手動同期**: 派生可能な値をプロパティに二重に持たせて手で揃える
- ❌ **1 件更新での `set_vec()`**: 全行再描画とスクロール位置喪失を招く
- ❌ **深いプロパティのバケツリレー**: 3 階層を超えるなら `global` を検討する
- ❌ **`global` の乱用**: 逆に何でも `global` に置くと依存関係が追えなくなる。
  上表の 5 つ以外を安易に追加しない
- ❌ **ピクセル値のハードコード**: `Theme` / `Layout` のトークンを使う
- ❌ **端末種別による分岐**: 幅で分岐する（[`responsive-layout.md`](./responsive-layout.md)）
- ❌ **`accessible-role` だけ付けて `accessible-action-default` を付けない**
- ❌ **`.slint` の `if` ブロック内で宣言した `id` を外側から参照する**（コンパイルエラー）

## パフォーマンス

- **仮想スクロール**: 大量リストは `ListView`
- **リストの仮想化を壊さない**: `ListView` の中身を `for` の外側で組み立てない。
  `interaction.rs` の `a_long_task_list_only_instantiates_visible_rows` が、
  タスク 500 件と 5000 件で生成される行数が増えないことを検証している
- **条件付きレンダリング**: `if` で不要な要素を生成しない（`visible: false` は生成される）
- **差分更新**: `Model` の更新は行単位の通知で行う
- **画像/SVG**: 頻繁に使うアイコンは `@image-url` で静的に埋め込む
- **アニメーション**: `animate` は必要な箇所のみ。低スペック端末での駒落ちに注意

## エラーとローディングの扱い

`AppState` に 3 状態を持たせ、View で出し分ける。

```slint
if AppState.loading: LoadingIndicator { }
if !AppState.loading && AppState.error != "": ErrorBanner { message: AppState.error; }
if !AppState.loading && AppState.error == "": TaskListView { }
```

楽観的更新の失敗時のロールバックは ViewModel の責務
（[`viewmodel-architecture.md`](./viewmodel-architecture.md) 参照）。

## テスト

- `.slint` 単体のテストは行わない。ロジックを持たないため
- ViewModel の単体テストでロジックを検証する
- レイアウトはブレークポイント境界のスクリーンショット比較で確認する
  （[`responsive-layout.md`](./responsive-layout.md) 参照）

## 関連ドキュメント

- [ViewModel アーキテクチャ](./viewmodel-architecture.md)
- [UI レイヤーアーキテクチャ](./layers.md)
- [レスポンシブレイアウト](./responsive-layout.md)
- [UI 実装規約 (rules)](../../rules/ui.md)
