# CLAUDE_ja.md

`CLAUDE.md` の日本語版（人間の読者向けミラー）。**内容は `CLAUDE.md` と同期させること。**

## 応答ガイドライン

- 常に **日本語** で応答する。
- このファイルを読み込んだら、まず `✅️ CLAUDE.md loaded` と発言してから指示に従う。

## プロジェクト

**Rust + Slint** で構築するネイティブタスク管理アプリ（単一プロセス、WebView なし、IPC なし）。
SvelteKit + Tauri 実装（`varubogu/flequit`）からの移植版。

- UI: Slint 1.x（`.slint` 宣言的 UI + Rust バインディング）、i18n は bundled translations
- コア: Rust（edition 2024）、Sea-ORM + SQLite、Automerge (CRDT)、Tokio、`tracing`
- 対象: Windows / macOS / Linux（Phase 1）+ Android / iOS（Phase 2）。Web は対象外。
- パッケージマネージャ: **Cargo のみ**。Node.js / Bun / npm への依存なし。

## 重要ルール

- 無関係なコードをユーザーの確認なしに変更しない。
- 正規表現・一括置換を行う場合、適用前に必ず差分を確認する。
- 「ファイル/ディレクトリが見つからない」エラーが出たら、まず `pwd` を確認する。
- cargo のワーカー数は必ず制限する: `cargo test -j 4`。
- `git commit` と `git push` はユーザーが行う。

## アーキテクチャ不変条件

CI（`./scripts/check-crate-deps.sh`）で検証される。破ってはならない。

- クレート依存方向:
  `flequit-types → flequit-model → flequit-repository → flequit-core →
  flequit-infrastructure-* → flequit-infrastructure → flequit-ui → flequit-app`
  （`flequit-platform` は `flequit-types` のみに依存する葉クレート）
- `#[cfg(target_os = ...)]` は **`flequit-platform` の内部にのみ** 書く。
- `.slint` は業務ロジックを持たず、ドメイン型を扱わない。
- `flequit-core` の facade を呼ぶのは ViewModel のみ。UI から repository を直接呼ばない。
- UI とコアは OS API を直接呼ばない。`flequit-platform` を経由する。
- UI スレッドをブロックしない。バックグラウンドからの UI 更新は
  `upgrade_in_event_loop()` 経由のみ。
- レスポンシブ分岐は **ウィンドウ幅** で行う。ターゲット OS で分岐しない。

## 参照先

- 残作業・優先度・確定済みの設計判断: `plans/plan.md`
- 設計 / ルール / 要件 / コマンド: `docs/ja/develop/{design,rules,requirements,commands.md}`
- `docs/ja/` が正本。`docs/en/` は未作成（別タスク）。
- タスク別のガイダンスは skill（`.claude/skills/`）にあり自動起動する。
  skill の内容をここに重複させない。
- `.codex/skills/` は `./scripts/sync-agent-skills.sh` により `.claude/skills/` から生成される。
  編集は `.claude/skills/` に対して行い、スクリプトを再実行すること。
  `.codex/skills/` を直接編集してはならない。
