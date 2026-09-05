# Flequit (Slint)

**Rust + [Slint](https://slint.dev)** で構築する、ローカルファーストのネイティブタスク管理アプリケーション。

[varubogu/flequit](https://github.com/varubogu/flequit)（SvelteKit + Tauri）を、
WebView と IPC を持たない Rust 単一プロセス構成へ移植したものです。

English: [README.md](./README.md)

## 状況

開発初期段階です。ドメイン・永続化・プラットフォーム層は整備済みで UI シェルは動作します。
元アプリとの機能同等性は作業中です。

| 領域 | 状態 |
| --- | --- |
| ドメイン / 永続化（SQLite + Automerge） | 移植元からそのまま流用 |
| プラットフォーム抽象化（`flequit-platform`） | デスクトップ実装済み、モバイルはスタブ |
| Slint UI シェル（サイドバー / 一覧 / 詳細、レスポンシブ） | 実装・配線済み |
| タスク追加・完了・タイトル/ノート編集・サブタスク完了 | 実装済み |
| タスク削除 | 保留（facade が `sea_orm` の型を呼び出し側に漏らしている） |
| 検索キーワード（`@today`・`#tag`）・繰り返し・設定画面 | 未着手 |

## 対応プラットフォーム

| プラットフォーム | フェーズ | 状態 |
| --- | --- | --- |
| Windows / macOS / Linux | 1 | 開発中 |
| Android / iOS | 2 | 構造のみ整備、実装は未着手 |
| Web | — | 対象外 |

UI は単一コードベースです。レイアウトは **ウィンドウ幅** で切り替わり、
ターゲット OS では分岐しません。デスクトップでウィンドウを縮めると
スマートフォンと同じレイアウトになります。

## 必要環境

- Rust（[`rust-toolchain.toml`](./rust-toolchain.toml) 参照。[mise](https://mise.jdx.dev/) を使えば自動で揃います）
- Linux は追加で `libxkbcommon-x11-0`、`libwayland-dev`、`libfontconfig1-dev`、`libdbus-1-dev` が必要

**Node.js / Bun / npm への依存はありません。**

## はじめかた

```sh
# ビルドと実行
cargo run -p flequit-app

# ログ付き
RUST_LOG=debug cargo run -p flequit-app
```

## 開発

```sh
cargo check --quiet                              # 型チェック
cargo clippy --all-targets -- -D warnings        # Lint
cargo fmt --all                                  # フォーマット
./scripts/test-prepare.sh                        # テスト事前準備（初回のみ）
cargo test -j 4                                  # テスト（-j 4 必須）
./scripts/check-crate-deps.sh                    # アーキテクチャ不変条件の検証
```

コマンド一覧: [`docs/ja/develop/commands.md`](./docs/ja/develop/commands.md)

## アーキテクチャ

```text
Slint UI (.slint)
    ↕  生成された Rust バインディング
ViewModel          crates/flequit-ui/src/viewmodels/
    ↓
Facade / Service   crates/flequit-core/
    ↓
Repository トレイト crates/flequit-repository/
    ↓
SQLite / Automerge crates/flequit-infrastructure-*/

flequit-platform   OS 抽象化（パス・通知・ダイアログ・Capability）
```

`./scripts/check-crate-deps.sh` が検証する不変条件:

- クレート依存は一方向のみ
- `#[cfg(target_os = ...)]` は `flequit-platform` の内部にのみ存在する
- `.slint` は業務ロジックを持たず、ドメイン型を扱わない
- `flequit-core` の facade を呼ぶのは ViewModel のみ
- ファイルパスをハードコードしない

## 残作業

残作業・優先度・確定済みの設計判断: [`plans/plan.md`](./plans/plan.md)

## ドキュメント

日本語が正本です: [`docs/ja/`](./docs/ja/)

- [アーキテクチャ](./docs/ja/develop/design/architecture.md)
- [技術スタック・クレート構成](./docs/ja/develop/design/tech-stack.md)
- [UI レイヤー](./docs/ja/develop/design/ui/layers.md)
- [レスポンシブレイアウト](./docs/ja/develop/design/ui/responsive-layout.md)
- [プラットフォーム抽象化](./docs/ja/develop/design/platform/platform-abstraction.md)
- [開発ワークフロー](./docs/ja/develop/rules/workflow.md)

## ライセンス

MIT
