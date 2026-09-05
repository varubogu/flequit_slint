# デプロイメント設計書

## 1. デプロイメント環境

### 1.1 デスクトップ配布

`cargo-packager` で各 OS のインストーラを生成する。

| OS | 形式 | 署名 |
| --- | --- | --- |
| Windows | `.msi` / `.exe`（NSIS） | Authenticode |
| macOS | `.dmg` / `.app` | Developer ID + notarization |
| Linux | `.deb` / `.rpm` / AppImage | GPG（リポジトリ配布時） |

配布経路:

- GitHub Releases（一次配布）
- Microsoft Store（Windows）
- Homebrew Cask（macOS）
- Flathub（Linux）

### 1.2 モバイル配布

| ストア | 対象 | 形式 |
| --- | --- | --- |
| Google Play ストア | Android | AAB（`xbuild` で生成） |
| App Store | iOS | IPA（Xcode Archive） |

- 自動アップデートはストアの機構に委ねる
- ベータ配布は Google Play の内部テスト / TestFlight を使用する

### 1.3 Web アプリケーション

現時点では対象外（`architecture.md` §3 参照）。

## 2. リリース管理

### 2.1 バージョニング

- セマンティックバージョニング採用
  - MAJOR.MINOR.PATCH 形式
  - 互換性のない変更: MAJOR
  - 機能追加: MINOR
  - バグ修正: PATCH
- 全プラットフォームでバージョンを揃える
  （workspace の `Cargo.toml` を正本とし、Android の `versionCode` /
  iOS の `CFBundleVersion` はビルド時に導出する）

### 2.2 リリースフロー

1. 開発ブランチでの開発
2. ステージングブランチでのテスト
3. メインブランチへのマージ
4. タグ付けとリリース作成
5. 各プラットフォームへの配布

### 2.3 リリース前チェックリスト

- 全テストの成功確認（`cargo test -j 4`）
- 全ターゲットのビルド確認（デスクトップ 3 OS + Android + iOS）
- 依存方向・cfg 隔離の検証（`./scripts/check-crate-deps.sh`）
- 翻訳の未対応エントリがないこと
- ブレークポイント境界のレイアウト確認
- パフォーマンス要件の充足
- セキュリティチェック（`cargo audit`）
- ドキュメント更新確認

## 3. アップデート戦略

### 3.1 自動アップデート

| プラットフォーム | 方式 |
| --- | --- |
| デスクトップ（GitHub Releases 経由） | アプリ内更新チェック + ダウンロード + 再起動適用 |
| デスクトップ（ストア経由） | ストアの自動アップデート |
| Android / iOS | ストアの自動アップデート |

- アプリ内更新は `Capability::SelfUpdate` で可否を判定する
  （ストア配布版では無効化する）
- ロールバック手段を用意する（旧バージョンのバイナリを保持）

### 3.2 段階的リリース

- カナリアリリース
  - ベータユーザー向け先行配布
  - フィードバック収集
  - 問題発生時の即時停止
- 段階的ロールアウト
  - ストアの段階公開機能を使用（Google Play / App Store）
  - 地域ごとの監視

## 4. データ管理

### 4.1 ローカルデータ

- ファイルベースデータストア（SQLite + Automerge）
- 保存先は `flequit-platform::paths::data_dir()` が返すディレクトリ
- 標準的なファイル形式採用
- 外部バックアップ対応（デスクトップは `.automerge` ファイルの直接コピーが可能）
- データ移行ツール提供

### 4.2 プラットフォーム間のデータ移行

- Automerge ファイルの互換性を全プラットフォームで維持する
- エクスポート / インポート機能を提供し、
  ファイル選択は `flequit-platform::file_dialog` 経由で行う
- モバイルはサンドボックス内のみのため、共有はエクスポート機能に依存する

### 4.3 同期データ

将来の同期サーバ導入時に定義する（`design/api/api.md` 参照）。

## 5. 監視と運用

### 5.1 アプリケーション監視

- エラー監視
  - クラッシュレポート収集
  - エラーログ分析
  - 使用統計収集
- パフォーマンス監視
  - 起動時間・フレーム描画時間
  - 主要操作のレスポンスタイム
  - リソース使用量
  - 同期状態

### 5.2 プラットフォーム別のログ

| プラットフォーム | 出力先 |
| --- | --- |
| デスクトップ | `flequit-platform::paths::log_dir()` 配下のローテーションファイル |
| Android | logcat（`tracing-android`）+ ファイル |
| iOS | OSLog + ファイル |

## 6. 障害対応

### 6.1 アプリケーション

- オフライン動作の保証
- データ破損防止機能（WAL モード、書き込み前の検証）
- 自動リカバリー機能
- エラーレポート送信（ユーザーの同意を得た上で）

### 6.2 モバイル固有

- 予告なきプロセス破棄への耐性（終了時保存に依存しない）
- ストレージ枯渇時の縮退動作
- 低メモリ時の Automerge ドキュメント解放

## 7. セキュリティ対策

### 7.1 アプリケーションセキュリティ

- コード署名（全プラットフォーム）
- 改ざん検知
- 安全な通信（TLS）
- モバイルはアプリサンドボックス内のみにデータを配置

### 7.2 デプロイメントセキュリティ

- CI/CD パイプラインの保護
- シークレット管理（署名鍵、ストア API キー）
- アクセス制御
- 監査ログ
- 依存クレートの脆弱性監査（`cargo audit` を CI で実行）

## 8. CI/CD

| ジョブ | 内容 |
| --- | --- |
| `lint` | `cargo fmt --check`, `cargo clippy -D warnings` |
| `test` | `cargo test -j 4`（Linux / macOS / Windows） |
| `deps` | `./scripts/check-crate-deps.sh`, `cargo audit` |
| `i18n` | `.pot` 差分チェック、未翻訳エントリ数の集計 |
| `build-mobile` | Android / iOS のクロスコンパイル確認 |
| `package` | タグ push 時に各 OS のインストーラを生成 |
