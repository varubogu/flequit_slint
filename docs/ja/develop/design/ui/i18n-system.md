# 国際化 (i18n) システム設計

Flequit は **Slint 組み込みの翻訳機構**（`@tr()` + bundled translations）で
リアルタイム言語切り替えを実現する。全 UI テキストは多言語対応し、
設定画面での言語選択後は即時反映される（再起動不要）。

> 実装の正本は `crates/flequit-ui/ui/`、`crates/flequit-ui/build.rs`、`i18n/` を参照。

## システム構成

- **翻訳機構**: Slint の `@tr()` マクロ + bundled translations
- **翻訳フォーマット**: gettext の `.po` / `.pot`
- **対応言語**: 英語（ベース）、日本語
- **抽出ツール**: `slint-tr-extractor`
- **バンドル**: `build.rs` の `slint_build` で実行ファイルへ埋め込む
- **実行時切替**: `slint::select_bundled_translation(lang)`

### なぜ bundled translations か

Slint は 2 方式を提供する。

| 方式 | 実行時切替 | 配布 | 採用 |
| --- | --- | --- | --- |
| gettext ランタイム | `LANGUAGE` 環境変数 + `update_all_translations()` | `.mo` ファイルを同梱・配置が必要 | ✗ |
| bundled translations | `select_bundled_translation()` | 実行ファイルに埋め込み | ✓ |

bundled translations を採用する理由:

- **モバイルで確実に動く**: Android / iOS では gettext のファイル配置が困難
- **配布が単純**: 翻訳ファイルの同梱漏れが起きない
- **切替が即時**: `select_bundled_translation()` の後、`update_all_translations()` の呼び出しも不要

### ディレクトリ構造

`.po` のファイル名は **gettext ドメイン名** であり、Slint は
`.slint` をコンパイルするクレート名（`flequit-ui`）をドメインとして使う。
そのためファイル名は `flequit-ui.po` でなければならず、
`flequit.po` では認識されない。

```text
i18n/
├── flequit-ui.pot                     # 抽出テンプレート（自動生成）
├── en/LC_MESSAGES/flequit-ui.po
└── ja/LC_MESSAGES/flequit-ui.po

crates/flequit-ui/
├── build.rs                        # with_bundled_translations("../../i18n")
└── ui/
    └── globals/
        └── i18n.slint              # 言語一覧と、Rust 由来コードの文言マッピング
```

## 使用パターン

### 基本

```slint
Text { text: @tr("Task title"); }
```

### 引数付き

`{0}` `{1}` の位置指定、または `{}` の順次指定を使う。

```slint
Text { text: @tr("Welcome, {0}!", user-name); }
```

### 複数形

`|` で単数形と複数形を区切り、`% <count 式>` で件数を渡す。件数は `{n}` で参照する。

```slint
Text { text: @tr("{n} task" | "{n} tasks" % task-count); }
```

日本語には複数形がないため、`ja.po` では単一形のみを定義する。

### 文脈の明示

同じ原文で意味が異なる場合は、`"Context" =>` で文脈を与える。
省略時はコンポーネント名が文脈になる。

```slint
Text { text: @tr("Sidebar" => "Today"); }     // サイドバーの期限ボタン
Text { text: @tr("Filter" => "Today"); }      // フィルタ条件の表示
```

## Rust 側の文言

`@tr()` は `.slint` の中でのみ使える。Rust が生成する文言は
**そのまま文字列を渡さず、コードを渡して `.slint` 側で翻訳する**。

```text
ViewModel        →  エラーコード / ステータスコードなどの識別子を property へ
ui/globals/i18n.slint →  コードを @tr() の文言へマップする pure function
View             →  I18n.error-message(AppState.error-code) を表示
```

これにより **翻訳ソースが `.po` の 1 か所に集約** される。

原則:

- ❌ Rust 側で `"タスクの保存に失敗しました"` のような文字列を組み立てない
- ✅ Rust 側は `ErrorCode::TaskSaveFailed` に相当する識別子を渡す
- ✅ 可変部分（タスク名など）は別プロパティで渡し、`.slint` で `@tr("...{0}...", name)` に埋める

例外: ログ出力（`tracing`）は翻訳対象外。英語固定とする。

## ロケール管理

### `global I18n`（`.slint`）

| プロパティ / 関数 | 内容 |
| --- | --- |
| `current-locale` | 現在の言語タグ（`"en"` / `"ja"`） |
| `available-locales` | 選択肢の一覧（タグと表示名） |
| `locale-changed(locale)` | 言語変更を Rust へ通知する callback |
| `error-message(code)` | エラーコード → 翻訳済み文言 |
| `status-label(status)` | ステータス → 翻訳済み表示名 |

### Rust 側の処理

言語切替時の処理順序:

1. `I18n.locale-changed` コールバックを受け取る
2. `slint::select_bundled_translation(&locale)` を呼ぶ
3. 設定を永続化する（`flequit-settings`）
4. `I18n.current-locale` を更新する

起動時:

1. 設定から保存済みロケールを読む
2. 未設定ならシステムロケールから判定し、未対応言語なら `en` にフォールバックする
3. **最初のコンポーネントを生成した後** に `select_bundled_translation()` を呼ぶ
   （Slint の制約。生成前に呼んでも反映されない）

## ワークフロー

### 新しい文言を追加する

1. `.slint` に `@tr("New text")` を書く
2. `.pot` を再生成する
   ```sh
   find crates/flequit-ui/ui -name '*.slint' | xargs slint-tr-extractor -o i18n/flequit-ui.pot
   ```
3. 各言語の `.po` へマージする
   ```sh
   msgmerge --update i18n/ja/LC_MESSAGES/flequit-ui.po i18n/flequit-ui.pot
   ```
4. `.po` を翻訳する
5. `cargo build` で再バンドルされる

### 日時・数値のロケール対応

- 日時フォーマットは翻訳ではなくユーザー設定で制御する
  （`design/ui/page/settings/datetime-format.md` 参照）
- タイムゾーン変換は表示層で行う（`design/data/data-model.md` の UTC ポリシー参照）

## メッセージ管理のベストプラクティス

### 原文の書き方

`@tr()` の原文（msgid）がそのまま英語 UI になる。以下を守る。

- 完全な文・句として書く。単語を連結して文を組み立てない
  - ❌ `@tr("Delete") + " " + @tr("task")`
  - ✅ `@tr("Delete task")`
- 語順が言語で変わるため、可変部分は必ずプレースホルダにする
- UI 上の文字数制約がある場合は文脈（`"Context" =>`）で補足する

### 文脈の付与

以下は必ず文脈を付ける。

- 1 語だけの文言（`Today`, `New`, `Open` など）
- ボタンラベルとメニュー項目で同じ語を使う場合
- 略語

### 翻訳漏れの検出

- CI で `.pot` を再生成し、コミット済みの `.pot` と差分がないことを確認する
- 各 `.po` の未翻訳エントリ（`msgstr ""`）数を集計し、増加時に警告する

## テスト

- ViewModel の単体テストは翻訳に依存しない
  （Rust 側は識別子しか扱わないため、翻訳のモックが不要）
- 言語切替の検証は、`select_bundled_translation()` 後に
  代表的なプロパティが変化することを確認する統合テストで行う
- レイアウト崩れの検出のため、各画面を `en` / `ja` の両方でスクリーンショット比較する
  （ドイツ語のような長い言語を追加する際はここが効く）

## Svelte 版からの変更点

| Svelte 版 | Slint 版 |
| --- | --- |
| Inlang Paraglide | Slint 組み込み翻訳 |
| `project.inlang/messages/{en,ja}.json` | `i18n/{en,ja}/LC_MESSAGES/flequit-ui.po` |
| キー参照（`m.task_title()`） | 原文参照（`@tr("Task title")`） |
| `reactiveMessage()` による再評価 | Slint が自動で再評価 |
| `bun run machine-translate` | `msgmerge` + 手動翻訳（機械翻訳は任意ツール） |
| ビルド時に TS を生成 | ビルド時に実行ファイルへバンドル |

**キー方式から原文方式に変わる点に注意**。
原文を変更すると別の msgid になるため、既存訳が外れる。
文言修正時は `.po` の該当エントリも更新する。

## 関連

- skill: `.claude/skills/i18n/SKILL.md`
- [Slint 設計パターン](./slint-patterns.md)
- [ViewModel アーキテクチャ](./viewmodel-architecture.md)
- [レスポンシブレイアウト](./responsive-layout.md)
