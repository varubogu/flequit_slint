# テスト要件

実装ルール (テストフレームワーク、配置、コマンド、設計原則) は [`../rules/testing.md`](../rules/testing.md) を参照。本書は **品質要件としてのテスト戦略** を定義する。

## CI/CD

GitHub Actions が push (main) と PR で実行する。ワークフローの実体は
`.github/workflows/` を参照。

| ジョブ | 内容 |
| --- | --- |
| `lint` | `cargo fmt --check`, `cargo clippy -D warnings` |
| `test` | `cargo test -j 4`（Linux / macOS / Windows） |
| `build` | 各ターゲットのビルド確認 |
| `audit` | `cargo audit`（push / PR + 毎週月曜） |
| `i18n` | `.pot` 差分チェック、未翻訳エントリ数の集計 |

自動デプロイとステージング環境は存在しない。配布はタグ push 時の手動リリース
（[`../design/deployment.md`](../design/deployment.md) 参照）。

## テスト種別と範囲

- **単体テスト**: ドメインロジック、Repository、ViewModel、Adapter、ユーティリティ
- **結合テスト**: Facade → Repository、Automerge と SQLite の整合性、
  UI 操作がハンドラへ届くこと（`crates/flequit-ui/tests/interaction.rs`）
- **システムテスト**: 実利用シナリオ（プロジェクト作成 → タスク追加 → 完了 → 削除）
- **将来**: 同期サーバ実装後にマルチアカウント同期・競合解決のシナリオを追加する

## 品質ゲート (PR マージ条件)

- 全テスト成功（`cargo test -j 4`）
- Lint エラーなし（`cargo clippy --all-targets -- -D warnings`）
- 依存方向・cfg 隔離の検証を通過（`./scripts/check-crate-deps.sh`）
- コードレビュー承認

**カバレッジの数値目標は設けない**。計測基盤（`cargo-llvm-cov` 等）を
導入しておらず、CI ゲートにもできないため、達成状況を誰も確認できない。
代わりに「新しい対話要素を追加したら `crates/flequit-ui/tests/interaction.rs` に
ケースを追加する」のような、**レビューで確認できる具体的なルール** で担保する
（[`../rules/testing.md`](../rules/testing.md) 参照）。

## テストデータセット

- 最小 (基本機能確認)
- 標準 (一般的な使用パターン)
- 大規模 (パフォーマンステスト用): タスク 10,000 件、プロジェクト 100 件、タグ 1,000 件

## テスト失敗時の対応

原因分析 → 再現手順の文書化 → 修正 → 再テスト。
CI が不安定な状態で放置されたテストは削除するか `#[ignore]` を付け、
理由をコメントに残す。
