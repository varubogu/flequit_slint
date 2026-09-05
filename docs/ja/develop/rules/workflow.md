# 開発ワークフロー

## 推奨手順

PR 運用の基本ルール:

- 変更の設計判断に関係する場合、ドキュメント変更を同梱（該当なしは N/A を明記）
- PR 説明に「適用パターン / 回避アンチパターン」を記述（PR テンプレ参照）
- レビューチェックリストを満たすこと
  （`docs/ja/develop/design/ui/slint-patterns.md`、
  `docs/ja/develop/design/backend/rust-guidelines.md` 参照）

手順の過程でエラーが発生した場合、解消後に次工程に進む。

## UI（Slint / ViewModel）のコード修正時

1. コード編集
2. `cargo check --quiet` - エラーがないかチェック（警告は一旦除く）
3. `cargo check --all-targets` - 警告がないかチェック
4. `cargo clippy --all-targets -- -D warnings` - リンター実行
5. `cargo fmt --all` - フォーマッター実行
6. ViewModel の単体テストケース作成
7. `cargo test -j 4 <test_name>` - 単体テスト（対象指定）実行
8. `cargo test -p flequit-ui -j 4` - UI クレート全テスト実行
9. レイアウト確認（ブレークポイント境界 599/600/1023/1024px）
10. `cargo test -j 4` - 全テスト実行

### 詳細手順

#### 1. コード編集

- **実装方針確認**: 関連する設計ドキュメント（`docs/ja/develop/design/ui/`）を確認
- **既存コード分析**: 修正対象ファイルと関連ファイルの構造を把握
- **段階的実装**: 小さな単位で実装し、都度動作確認
- **層の遵守**: `.slint` にロジックを書かない、ViewModel から facade のみを呼ぶ
- **動作しない場合の対処**:
  - `slint-viewer` で該当 `.slint` を単体プレビューし、レイアウトを切り分ける
  - `RUST_LOG=debug cargo run -p flequit-app` でログを確認
  - プロパティが更新されない場合、Rust 側の setter を呼び忘れていないか確認
  - バックグラウンドからの更新は `upgrade_in_event_loop` を経由しているか確認

#### 6. ViewModel 単体テストケース作成

- **テスト対象の特定**: 作成・修正した ViewModel の公開メソッドを洗い出し
- **テストケース設計**:
  - 正常系: 期待される入力に対する `Model` / 状態の変化
  - 異常系: facade がエラーを返した際のロールバック
  - エッジケース: 空リスト、存在しない ID、境界値
- **Slint ウィンドウを生成しない**: `VecModel` と ViewModel の状態を直接検証する
- **テストが書けない場合の対処**:
  - 依存が複雑 → facade とプラットフォームをモック trait で注入
  - 非同期処理 → `#[tokio::test]` を使用

#### 9. レイアウト確認

- ウィンドウ幅を 599 / 600 / 1023 / 1024px に変えてレイアウト崩れがないか確認する
- `Compact` で一覧と詳細が排他表示になること、戻る導線があることを確認する
- タップ領域が 48px 以上確保されているか確認する
- 詳細は `docs/ja/develop/design/ui/responsive-layout.md` を参照

## コア / インフラのコード修正時

1. コード編集
2. `cargo check --quiet` - エラーがないかチェック（警告は一旦除く）
3. `cargo check --all-targets` - 警告がないかチェック
4. `cargo clippy --all-targets -- -D warnings` - リンター実行
5. `cargo fmt --all` - フォーマッター実行
6. `cargo test -j 4 <unit test name>` - 単体テスト（対象指定）実行
7. 結合テストケース作成
8. `cargo test -j 4 --test <integration test name>` - 結合テスト（対象指定）実行
9. `cargo test -j 4` - 全テスト実行

### 詳細手順

#### 1. コード編集

- **設計ドキュメント確認**: `docs/ja/develop/design/backend/` の関連ドキュメントを確認
- **既存コード分析**: 修正対象モジュールの依存関係と構造を把握
- **エラーハンドリング重視**: Rust の型システムを活用した安全な実装
- **メモリ安全性確保**: 所有権システムを意識した実装
- **動作しない場合の対処**:
  - コンパイラエラーメッセージの詳細確認
  - `cargo check` による段階的なエラー解消
  - `dbg!` マクロや `tracing` によるデバッグ出力
  - 単体テストでの小さな単位での動作確認

#### 6. 単体テストケース作成

- **テスト対象の特定**: 作成・修正した関数、メソッドの動作確認
- **テストケース設計**:
  - 正常系: 期待される入力に対する正しい出力
  - エラー処理: `Result` 型の Err 系の適切なハンドリング
  - 境界値テスト: 最大値、最小値、空データ等での動作確認
- **テストが書けない場合の対処**:
  - 外部依存が多い → トレイトによる抽象化とモック化
  - 非同期処理 → `#[tokio::test]` によるテスト環境整備
  - ファイル I/O → `tempfile` クレートによる一時ファイル利用

#### 7. 結合テストケース作成

- **結合テスト対象の特定**: モジュール間の連携、データベースとの結合等
- **テストシナリオ設計**: 実際の使用パターンに沿ったテストケース
- **テスト環境構築**: テスト用データベース、設定ファイル等の準備
- **結合テストが複雑になる場合の対処**:
  - データベース依存 → テスト用のマイグレーション整備（`./scripts/test-prepare.sh`）
  - 外部サービス依存 → `wiremock` 等によるモック化
  - 設定ファイル依存 → テスト専用設定の作成

## プラットフォーム対応コードの修正時

1. `flequit-platform` の trait（共通インターフェース）を先に定義・変更する
2. デスクトップ実装を書き、`cargo test -j 4 -p flequit-platform` で検証する
3. Android / iOS 実装を書く
4. `Capability` の表を更新する（非対応プラットフォームがある場合）
5. `docs/ja/develop/design/platform/platform-abstraction.md` を更新する
6. `./scripts/check-crate-deps.sh` で cfg の漏れがないことを確認する
7. クロスコンパイルでビルド確認する

```bash
cargo check --target aarch64-linux-android
cargo check --target aarch64-apple-ios   # macOS のみ
```

## 両方のコード修正時

コア → UI の順番で推奨手順を実施する（依存方向に沿うため）。

## クレート構成を変更した時

1. `Cargo.toml`（workspace とクレート個別）を更新
2. `./scripts/check-crate-deps.sh` を実行して依存方向違反がないことを確認
3. `docs/ja/develop/design/tech-stack.md` と
   `docs/ja/develop/rules/file-structure.md` の構成図を更新
4. `cargo test -j 4` を実行

## i18n 文言を追加・変更した時

1. `.slint` に `@tr()` を追加または修正
2. `.pot` を再生成し、各 `.po` へ `msgmerge` でマージ
3. 翻訳を記入
4. `cargo build` でバンドルを更新
5. `en` / `ja` の両方でレイアウト崩れがないか確認

原文（msgid）を変更すると既存訳が外れる点に注意。詳細は
`docs/ja/develop/design/ui/i18n-system.md` を参照。
