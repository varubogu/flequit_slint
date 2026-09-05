# 開発コマンド一覧

正本は各 `Cargo.toml` と `scripts/`。本書は頻用コマンドのみ抜粋する。
Node.js / Bun への依存はない。

## 基本（全体）

| 用途 | コマンド |
| --- | --- |
| 構文チェック | `cargo check --quiet` |
| 警告チェック | `cargo check --all-targets` |
| Lint | `cargo clippy --all-targets -- -D warnings` |
| Format | `cargo fmt --all` |
| Format チェック | `cargo fmt --all -- --check` |
| テスト | `cargo test -j 4` （`-j 4` 必須） |
| 対象を絞ったテスト | `cargo test -j 4 <test_name>` |
| クレート単位のテスト | `cargo test -p flequit-core -j 4` |
| 依存方向・cfg 隔離の検証 | `./scripts/check-crate-deps.sh` |

> `cargo test` は必ず `-j 4` でワーカー数を制限する。SQLite のテスト DB 競合と
> メモリ枯渇を避けるため。

## アプリの実行

| 用途 | コマンド |
| --- | --- |
| デスクトップ実行 | `cargo run -p flequit-app` |
| ログレベル指定 | `RUST_LOG=debug cargo run -p flequit-app` |
| リリースビルド | `cargo build --release -p flequit-app` |

> アプリの起動はユーザーが行っている場合がある。エージェントが実行する前に確認すること。

## UI（Slint）

| 用途 | コマンド |
| --- | --- |
| 単一 `.slint` のライブプレビュー | `slint-viewer crates/flequit-ui/ui/views/task-list/task-list.slint` |
| `slint-viewer` の導入 | `cargo install slint-viewer` |

## 国際化 (i18n)

| 用途 | コマンド |
| --- | --- |
| 抽出ツールの導入 | `cargo install slint-tr-extractor` |
| `.pot` 再生成 | `find crates/flequit-ui/ui -name '*.slint' \| xargs slint-tr-extractor -o i18n/flequit-ui.pot` |
| `.po` へのマージ | `msgmerge --update i18n/ja/LC_MESSAGES/flequit-ui.po i18n/flequit-ui.pot` |
| 未翻訳の確認 | `msgfmt --statistics -o /dev/null i18n/ja/LC_MESSAGES/flequit-ui.po` |

翻訳は `cargo build` 時に実行ファイルへバンドルされる。個別のビルド手順は不要。

## テスト事前準備

```bash
./scripts/test-prepare.sh          # 下記をまとめて実行
./scripts/test-prepare.sh automerge  # Automerge 用テストディレクトリ作成
./scripts/test-prepare.sh db         # SQLite テスト DB 準備
./scripts/test-prepare.sh db --force # SQLite テスト DB を作り直す
```

SQLite テスト DB のマイグレーションは以下でも実行できる。

```bash
cargo run -p flequit-infrastructure-sqlite --bin migration_runner -- .tmp/tests/test_database.db
```

## モバイルビルド

### Android

| 用途 | コマンド |
| --- | --- |
| ツール導入 | `cargo install --git https://github.com/rust-mobile/xbuild.git` |
| 環境診断 | `x doctor` |
| 接続デバイス一覧 | `x devices` |
| 実機/エミュレータで実行 | `x run --device <id>` |
| APK 生成 | `x build --platform android --arch arm64 --format apk --release` |

### iOS

macOS + Xcode が必要。

| 用途 | コマンド |
| --- | --- |
| ターゲット追加 | `rustup target add aarch64-apple-ios aarch64-apple-ios-sim` |
| ライブラリビルド | `cargo build --target aarch64-apple-ios --release` |
| Xcode プロジェクト生成 | `xcodegen generate --spec mobile/ios/project.yml` |

## 配布パッケージ（デスクトップ）

| 用途 | コマンド |
| --- | --- |
| ツール導入 | `cargo install cargo-packager --locked` |
| パッケージ生成 | `cargo packager --release` |

## 推奨実行順（修正後の確認フロー）

詳細手順は `docs/ja/develop/rules/workflow.md` 参照。サマリ:

- **UI 修正時**: `cargo check --quiet` → `cargo clippy` → 個別 `cargo test -j 4 <name>`
  → `cargo test -p flequit-ui -j 4` → 必要ならブレークポイント確認
- **コア/インフラ修正時**: `cargo check --quiet` → `cargo clippy` →
  個別 `cargo test -j 4 <name>` → `cargo test -j 4`
- **両方修正時**: コア → UI の順で実施（依存方向に沿う）
- **クレート構成を変えた時**: `./scripts/check-crate-deps.sh` を必ず実行
