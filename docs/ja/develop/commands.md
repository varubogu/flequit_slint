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

> Windows では `CARGO_INCREMENTAL=0` を付ける。インクリメンタルディレクトリの
> 書き込みが拒否されると rustc が `STATUS_STACK_BUFFER_OVERRUN` で落ちる。

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
| `.pot` 再生成 | `find crates/flequit-ui/ui -name '*.slint' \| sort \| xargs slint-tr-extractor -o i18n/flequit-ui.pot` |
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

手順の詳細は `mobile/android/README.md` と `mobile/ios/README.md`。

### Android

Gradle が `FlequitActivity.java` のコンパイルと APK パッケージングを担い、
`preBuild` から `cargo-ndk` が `libflequit_app.so` を生成する。

| 用途 | コマンド |
| --- | --- |
| ターゲット追加 | `rustup target add aarch64-linux-android x86_64-linux-android` |
| ツール導入 | `cargo install cargo-ndk` |
| クロスコンパイル確認 | `cargo check --target aarch64-linux-android -p flequit-app --features android` |
| APK 生成 | `cd mobile/android && ./gradlew assembleDebug` |
| インストール | `cd mobile/android && ./gradlew installDebug` |
| ログ確認 | `adb logcat -s flequit` |

> `--features android` は Android ターゲット指定と併用でしか通らない。
> `slint::android` 自体が `target_os = "android"` で閉じているため。

### iOS

macOS + Xcode が必要。Xcode の pre-build スクリプトが `cargo build` を呼ぶので、
通常は Xcode からのビルドだけでよい。

| 用途 | コマンド |
| --- | --- |
| ターゲット追加 | `rustup target add aarch64-apple-ios aarch64-apple-ios-sim` |
| ツール導入 | `brew install xcodegen` |
| クロスコンパイル確認 | `cargo check --target aarch64-apple-ios -p flequit-app --features ios` |
| Xcode プロジェクト生成 | `cd mobile/ios && xcodegen generate` |

## Web ビルド（UI デモ）

`crates/flequit-web` は共有の `.slint` をサンプルデータで描画するプレビュー。
永続化は無い。詳細は `web/README.md`。

| 用途 | コマンド |
| --- | --- |
| ターゲット追加 | `rustup target add wasm32-unknown-unknown` |
| ツール導入 | `cargo install wasm-bindgen-cli` |
| wasm ビルド | `cargo build --release -p flequit-web --target wasm32-unknown-unknown` |
| JS グルー生成 | `wasm-bindgen --target web --no-typescript --out-dir web/pkg target/wasm32-unknown-unknown/release/flequit_web.wasm` |
| 配信 | `python3 -m http.server --directory web 8080` |
| ツール無しでの検証 | `cargo test -j 4 -p flequit-web`（ホスト向けにビルドされる） |

## 依存関係の監査

| 用途 | コマンド |
| --- | --- |
| ツール導入 | `cargo install cargo-audit --locked` |
| 監査 | `cargo audit` |

脆弱性があれば失敗し、メンテナンス終了（unmaintained）は警告のまま通す。
除外は `.cargo/audit.toml` に理由つきで書く。CI は `.github/workflows/audit.yml`
が push / PR と毎週月曜に実行する。

## 配布パッケージ（デスクトップ）

| 用途 | コマンド |
| --- | --- |
| ツール導入 | `cargo install cargo-packager --locked` |
| パッケージ生成 | `cargo packager --release` |

## 修正後の確認フロー

[`rules/workflow.md`](./rules/workflow.md) が正本。
