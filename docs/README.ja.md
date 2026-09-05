# Flequit (Slint) ドキュメント

Flequit の Slint 実装版のドキュメントです。

## 言語

- [日本語ドキュメント](./ja/) — **source of truth**
- English documentation — 未整備（`docs/en/` は ja が固まった後に追従）

## ドキュメント構造

```text
docs/ja/
├── roadmap.md
└── develop/
    ├── commands.md              # 開発コマンド一覧
    ├── design/                  # 設計ドキュメント
    │   ├── architecture.md      # 全体アーキテクチャ
    │   ├── tech-stack.md        # 技術スタック・クレート構成
    │   ├── testing.md           # テスト環境
    │   ├── deployment.md        # ビルド・配布
    │   ├── error-handling.md
    │   ├── api/                 # 将来の同期サーバ API
    │   ├── backend/             # Rust 設計・トランザクション
    │   ├── data/                # データモデル・Automerge・エンティティ
    │   ├── platform/            # プラットフォーム抽象化
    │   └── ui/                  # Slint UI・ViewModel・レスポンシブ・i18n
    ├── requirements/            # 品質要件
    └── rules/                   # コーディングルール
```

## プロジェクト概要

Flequit は、プロジェクト管理とタスクコラボレーションをサポートする
**Rust + Slint によるネイティブタスク管理アプリケーション** です。

- 対応プラットフォーム: Windows / macOS / Linux / Android / iOS
- ローカルファースト（SQLite）で動作し、
  AutoMerge (CRDT) ベースのデータ管理により将来の同期時の競合を防ぎます
- WebView を持たない Rust 単一プロセス構成です

本リポジトリは、SvelteKit + Tauri で実装された
[flequit](https://github.com/varubogu/flequit) を Slint へ移植したものです。
移植にあたっての設計上の対応関係は、各設計ドキュメント末尾の
「Svelte 版からの変更点」表を参照してください。

## 読む順序

初めての場合は以下の順で読むことを推奨します。

1. [アーキテクチャ](./ja/develop/design/architecture.md) — 全体像
2. [技術スタック](./ja/develop/design/tech-stack.md) — クレート構成と依存方向
3. [UI レイヤー](./ja/develop/design/ui/layers.md) — UI の責務分離
4. [開発コマンド](./ja/develop/commands.md) — 手を動かすとき
5. [ワークフロー](./ja/develop/rules/workflow.md) — 修正時の手順

## 貢献

- `docs/ja/` を正本として編集してください
- ドキュメント編集ルールは
  [documentation.md](./ja/develop/rules/documentation.md) を参照してください
