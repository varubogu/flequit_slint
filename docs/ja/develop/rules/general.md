# 全般的なコーディングルール

## ファイル構成

### 基本原則

- **単一責任原則**: 1 ファイル 1 機能
- **ファイルサイズ**: 200 行超過で必須分割、100 行でも分割検討

### 命名規則

- **`.slint` ファイル**: ケバブケース（`task-item.slint`）
- **Rust ファイル**: スネークケース（`task_viewmodel.rs`）
- **Slint の component / struct / enum**: パスカルケース（`TaskItem`）
- **Slint の property / callback**: ケバブケース（`is-selected`, `toggle-completed`）
  - Rust 側の生成 API では自動的にスネークケースになる（`set_is_selected`, `on_toggle_completed`）
- **その他**: Rust 標準規約に準拠

## 国際化対応

- 全ての UI に関わるテキストは多言語対応を行う
- 設定画面で UI 言語を選択可能で、選択後は即時反映される（再起動不要）
- 文言は Slint の `@tr()` で記述し、翻訳は `i18n/*/LC_MESSAGES/flequit-ui.po` で管理する

### 使用方法

```slint
Text { text: @tr("Task title"); }
Text { text: @tr("Welcome, {0}!", user-name); }
Text { text: @tr("{n} task" | "{n} tasks" % task-count); }
```

Rust 側で文言を組み立ててはならない。識別子を UI へ渡し、`.slint` 側で `@tr()` にマップする。
詳細は `docs/ja/develop/design/ui/i18n-system.md` を参照。

## プラットフォーム対応

- 対応プラットフォーム: Windows / macOS / Linux / Android / iOS
- **UI の分岐は画面幅で行う**。端末種別・OS で分岐しない
  （`docs/ja/develop/design/ui/responsive-layout.md`）
- **OS API は `flequit-platform` 経由でのみ呼ぶ**。
  `#[cfg(target_os = ...)]` は `flequit-platform` の内部にのみ書く
  （`docs/ja/develop/design/platform/platform-abstraction.md`）
- 未対応機能は `Capability` で判定し、UI から該当項目を消す

## 開発制約

- **UI スレッドをブロックしない**: 同期 I/O・重い計算は `tokio::spawn` /
  `spawn_blocking` へ委譲する
- **バックグラウンドから UI を触らない**: `slint::Weak::upgrade_in_event_loop()` を経由する
- **終了時保存に依存しない**: モバイルは予告なくプロセスが破棄される。
  永続化は操作のたびに完了させる
- **テストタイムアウト**: ファイル内テスト件数 × 1 分
- **cargo test は必ず `-j 4`** でワーカー数を制限する
