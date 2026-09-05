# レスポンシブレイアウト設計

Flequit は Windows / macOS / Linux / Android / iOS を **単一の UI コードベース** で対応する。
端末種別ではなく **画面幅** を基準にレイアウトを切り替える。

> 実装の正本は `crates/flequit-ui/ui/globals/layout.slint` と各 `ui/views/` を参照。

## 基本方針

1. **幅で分岐する**。`target_os` やモバイル判定でレイアウトを変えない
   - デスクトップでウィンドウを縮めた場合もスマートフォンと同じレイアウトになる
   - テストがデスクトップ上で完結する
2. **分岐は Slint 側に閉じる**。ViewModel はレイアウトを知らない
3. **入力手段は幅と独立**。タッチ操作は全プラットフォームで成立させる
   （デスクトップのタッチスクリーンも対象）

## ブレークポイント定義

`ui/globals/layout.slint` で単一の global singleton として定義する。

| 名称 | 条件 | 想定端末 |
| --- | --- | --- |
| `Compact` | `width < 600px` | スマートフォン（縦） |
| `Medium` | `600px <= width < 1024px` | タブレット、スマートフォン（横）、小さいウィンドウ |
| `Expanded` | `width >= 1024px` | デスクトップ |

`Layout` global が公開するプロパティ:

| プロパティ | 型 | 内容 |
| --- | --- | --- |
| `window-width` | `length` | ルートウィンドウから注入される現在幅 |
| `is-compact` | `bool` | `window-width < 600px` |
| `is-medium` | `bool` | `600px <= window-width < 1024px` |
| `is-expanded` | `bool` | `window-width >= 1024px` |
| `touch-target` | `length` | 最小タップ領域（Compact: 48px / それ以外: 32px） |
| `content-padding` | `length` | 画面余白（Compact: 12px / Medium: 16px / Expanded: 24px） |

`window-width` は `main.slint` のルート `Window` が自身の `width` を代入する。
各コンポーネントは `Layout.is-compact` 等を参照するだけでよい。

## 画面別のレイアウト切替

### 全体構成

| ブレークポイント | サイドバー | タスク一覧 | タスク詳細 |
| --- | --- | --- | --- |
| `Expanded` | 常時展開（幅可変・リサイズ可） | 表示 | 表示（2 ペイン） |
| `Medium` | アイコンのみ（折りたたみ、展開可） | 表示 | 表示（2 ペイン） |
| `Compact` | オーバーレイ（ハンバーガーで開閉） | 表示 | タスク選択時に全画面へ切替 |

`Compact` では「一覧」と「詳細」を排他表示とし、詳細画面に戻るボタンを設ける。
移動状態は `AppState.active-pane`（`List` / `Detail`）で管理する。

### サイドバー

- `Expanded`: `<アイコン + 名前>` を左寄せで表示。折りたたみボタンでアイコンのみに切替可能
- `Medium`: 既定でアイコンのみ。クリックで一時的にオーバーレイ展開
- `Compact`: 既定で非表示。ヘッダーのハンバーガーボタンでオーバーレイ表示。
  背景タップまたは項目選択で閉じる

プロジェクトにアイコン未設定の場合、折りたたみ時はプロジェクト名の先頭 2〜3 文字を表示する
（`design/ui/page/main/main.md` の仕様を維持）。

### タスク詳細

- `Expanded` / `Medium`: 右ペインに常設
- `Compact`: 一覧から遷移する全画面。戻るボタンで一覧へ復帰

### 設定画面

- `Expanded` / `Medium`: 左にカテゴリ、右に設定項目の 2 ペインモーダル
- `Compact`: カテゴリ一覧 → 項目一覧の 2 階層遷移。戻るボタンで上位へ

### ダイアログ

- `Expanded` / `Medium`: 中央配置のモーダルダイアログ
- `Compact`: 画面下部から立ち上がるボトムシート、または全画面ダイアログ

## タッチ操作の要件

| 項目 | 要件 |
| --- | --- |
| 最小タップ領域 | `Compact` で 48x48px 以上（`Layout.touch-target`） |
| 要素間の間隔 | 隣接する操作要素は 8px 以上離す |
| ホバー依存の禁止 | ホバーでしか現れない操作を作らない。常時表示か長押しメニューにする |
| スクロール | 慣性スクロールを前提とし、スクロール領域内に横スワイプ操作を重ねない |
| 長押し | コンテキストメニューは右クリックと長押しの両方で開く |
| セーフエリア | ノッチ・ホームインジケータを避ける余白をルートで確保する |

## 実装パターン

### 条件付きレイアウト

```slint
// ブレークポイントによる要素の出し分け
if Layout.is-expanded: HorizontalLayout {
    TaskListView { }
    TaskDetailView { }
}
if !Layout.is-expanded && AppState.active-pane == Pane.List: TaskListView { }
if !Layout.is-expanded && AppState.active-pane == Pane.Detail: TaskDetailView { }
```

### `states` によるプロパティ切替

同一要素の寸法や配置だけを変える場合は `states` を使う。
アニメーションを付けられるため、サイドバーの開閉に適する。

```slint
states [
    collapsed when Layout.is-compact: {
        sidebar.width: 0px;
    }
    icon-only when Layout.is-medium: {
        sidebar.width: 62px;
    }
]
```

### 禁止パターン

- ❌ ViewModel 側で `is_mobile` を判定して UI へ渡す
- ❌ `.slint` を「デスクトップ用」「モバイル用」に分岐して二重管理する
- ❌ ピクセル値のハードコード（`Layout` / `Theme` のトークンを使う）

## テスト方針

- **ブレークポイント境界**: 599px / 600px / 1023px / 1024px の 4 点でレイアウトを確認する
- **デスクトップ上での検証**: ウィンドウリサイズで `Compact` 相当を再現できるため、
  レイアウト検証の大半は実機なしで実施できる
- **実機検証**: セーフエリア、慣性スクロール、ソフトキーボードによる領域圧迫、
  長押しメニューは実機（Android / iOS）で確認する
- **回帰**: 画面ごとに 3 ブレークポイントのスクリーンショットを取得して比較する

## 関連ドキュメント

- [UI レイヤーアーキテクチャ](./layers.md)
- [Slint 設計パターン](./slint-patterns.md)
- [メイン画面仕様](./page/main/main.md)
- [プラットフォーム抽象化](../platform/platform-abstraction.md)
- [ユーザビリティ要件](../../requirements/usability.md)
