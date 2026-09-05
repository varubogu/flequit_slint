# Flequit (Slint) 移植計画

SvelteKit + Tauri 版 [`varubogu/flequit`](https://github.com/varubogu/flequit) から
Rust + Slint への移植における残作業の記録。

- 最終更新: 2026-09-06
- 正本: 本ファイル。設計の詳細は `docs/ja/` を参照する
- 完了した項目はチェックを入れ、判断が変わった項目は理由を残す

---

## 1. 現状サマリ

| 領域 | 状態 |
| --- | --- |
| ドキュメント (`docs/ja/`) | 移植完了（48 ファイル）。`docs/en/` は未作成 |
| エージェント設定 | 完了（Claude / Codex / Cursor、同期スクリプト + CI 検証つき） |
| Rust クレート（ドメイン・永続化） | 移植元からそのまま流用。Tauri 依存なし |
| `flequit-platform` | デスクトップ実装済み。Android / iOS はスタブ |
| Slint UI シェル | サイドバー / タスク一覧 / タスク詳細、レスポンシブ 3 段階 |
| タスク操作 | 追加・完了・タイトル/ノート編集・サブタスク完了 |
| 検証 | `fmt` / `clippy -D warnings` / 226 tests / 依存不変条件 / i18n すべて green |

### 検証コマンド

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
./scripts/check-crate-deps.sh
./scripts/sync-agent-skills.sh --check
./scripts/test-prepare.sh && cargo test -j 4 --workspace
```

---

## 2. 確定済みの設計判断

再検討の前にここを読むこと。覆す場合は理由を追記する。

| 判断 | 内容 | 経緯 |
| --- | --- | --- |
| Slint 単体 | Tauri は併用しない | Tauri 2 は WebView 前提で UI 差し替え不可。`tauri-plugin-*` も `tauri::Runtime` 依存 |
| クレートは流用 | ドメイン・永続化はコピーして再利用 | 移植元に Tauri 依存が無く、そのまま動いた |
| ja 先行 | `docs/ja/` のみ整備 | `docs/en/` は実装が固まってから別タスク |
| PC 先行 | モバイルは Phase 2 | 設計・抽象化は最初から両対応で記述済み |
| 幅で分岐 | レスポンシブは OS 判定を使わない | デスクトップ上でレイアウト検証が完結する |
| Web 版は対象外 | WASM ターゲットも当面扱わない | 同期サーバの設計が未確定 |

---

## 3. P0 — ブロッカー

### 3.1 削除系 facade からストレージ固有型を除去する

- **問題**: `task_facades::delete_task` が
  `TransactionManager<Transaction = sea_orm::DatabaseTransaction>` の境界を要求する。
  ViewModel から呼ぶと `flequit-ui` が SQLite の型を知ることになり、
  `scripts/check-crate-deps.sh` が守る不変条件に反する
- **現状**: UI の削除ボタンは押すとエラー表示になる（`app.rs` の `on_delete_task`）
- **対応**: トランザクション境界を `flequit-infrastructure` の内側に隠し、
  facade の公開シグネチャを `Result<T, ServiceError>` に揃える
- **影響**: `flequit-core/src/facades/{task,subtask,project,tag}_facades.rs`、
  `flequit-infrastructure`
- **完了条件**: UI からタスク・サブタスク・プロジェクト・タグを削除でき、
  依存チェックが通ること
- 参照: `docs/ja/develop/design/ui/core-bridge.md` の「現状の制約: 削除系 facade」

### 3.2 facade の戻り値を `Result<T, ServiceError>` へ移行する

- **問題**: facade が Tauri 時代の名残で `Result<T, String>` を返す。
  ViewModel 側でエラー種別を判別できず、i18n コードへ正確にマップできない
- **現状**: すべて `task.save-failed` 等の粗いコードに丸めている
- **対応**: `handle_service_error` を廃止し、型付きエラーをそのまま返す
- **影響**: 全 facade（13 ファイル）と ViewModel のエラー変換
- **完了条件**: 「見つからない」「検証エラー」「ストレージ障害」を UI が区別して表示できる
- 参照: `docs/ja/develop/design/backend/rust-guidelines.md` の「エラーハンドリング」

---

## 4. P1 — ver1.0 相当の機能

ロードマップ ver1.0（`docs/ja/roadmap.md`）に必要な UI 機能。

### 4.1 タスク管理

- [ ] サブタスク作成（`Actions.add-subtask` は宣言済み・ハンドラ未登録）
- [ ] タスク削除（P0 3.1 の完了が前提）
- [ ] 期限の設定・編集（日時ピッカーが必要 → 4.5）
- [ ] 優先度の設定（`TaskPriority` は表示のみ。変更 UI が無い）
- [ ] ステータス変更（完了トグル以外の遷移）
- [ ] タスクのソート（期限 / 優先度 / 名前 / 手動並び替え）
- [ ] ドラッグ&ドロップによる並び替えとリスト間移動

### 4.2 プロジェクト / タスクリスト管理

- [ ] プロジェクトの作成・リネーム・削除・アーカイブ
- [ ] タスクリストの作成・リネーム・削除
- [ ] プロジェクトの色設定（`ProjectItem.color` は渡しているが未描画）

### 4.3 検索

- [ ] キーワード構文の解析
  - 期限: `@today` / `@今日` / `@overdue` / `@3days` ほか
  - 属性: `@project:` / `@list:` / `@task:` / `@note:` / `@subtask:`
  - タグ: `#tag`
  - **現状は部分一致のみ。`@`/`#` 始まりは「フィルタなし」として無視している**
    （`app.rs` の `matches_query`）
- [ ] 入力候補の表示（`@` / `#` 入力時）
- [ ] 期限フィルタボタンの件数表示（`DueFilterItem.count` が常に 0）
- 参照: `docs/ja/develop/design/ui/page/main/main.md`

### 4.4 タグ

- [ ] タグの作成・編集・削除 UI
- [ ] タスクへのタグ付け / 解除
- [ ] タスク行へのタグ表示
      （`adapters/task.rs` の `tag_labels` が常に空。タグ名の解決が未実装）
- [ ] タグブックマーク（サイドバー固定表示）

### 4.5 日時

- [ ] 日付・時刻ピッカーコンポーネント
- [ ] 日時フォーマット設定画面
      （設計は `docs/ja/develop/design/ui/page/settings/datetime-format.md` に完備）
- [ ] タイムゾーン設定の反映
      （現状 `DisplayTimezone::System` 固定。設定値を読んでいない）

### 4.6 繰り返し

- [ ] 繰り返しルールの設定 UI（日 / 週 / 月 / 年）
- [ ] 繰り返しプレビュー
- facade は実装済み（`recurrence_facades`: 18 関数）

### 4.7 リマインダー

- [ ] タスクへのリマインダー設定 UI（任意数）
- [ ] `flequit-platform::notification` への接続
- [ ] 通知許可のリクエスト導線（設定画面）

### 4.8 設定画面

- [ ] 設定モーダルの実装（`Actions.open-settings` はログ出力のみ）
  - 基本設定（曜日開始日、期限ボタンの表示有無、期限ボタン追加）
  - 外観設定（テーマ、フォント、フォントサイズ、配色）
  - アカウント設定
- [ ] `Capabilities` による項目の出し分け（設計済み・未実装）
- [ ] `Compact` での 2 階層遷移
- 参照: `docs/ja/develop/design/ui/page/settings/settings.md`

### 4.9 テーマ

- [ ] テーマ切替の実装
      **`Theme.dark` が Rust から一度も設定されておらず、常にライトテーマ**
- [ ] OS のダークモード追従（`ThemeMode.system`）
- [ ] 設定の永続化（`flequit-settings`）

### 4.10 UI シェルの未完部分

- [ ] 自動スクロール（`AppState.scroll-to-index` は宣言のみで消費側が無い）
- [ ] サイドバー折りたたみをデスクトップで機能させる
      （現在 `Layout.is-medium` から導出しており、ボタンを押しても変化しない）
- [ ] アカウントボタン（サイドバーフッター、メニュー表示）
- [ ] タスク一覧の空状態からの導線改善

---

## 5. P2 — 品質・基盤

### 5.1 アクセシビリティ

- [ ] キーボード操作の実装（要件は全機能をキーボードで操作可能）
  - タスク間移動、ペイン間移動、ショートカット（Ctrl+N ほか）
  - vim モード（オプション）
- [ ] フォーカス表示とフォーカストラップ
- [x] `accessible-role` / `accessible-action-default` の付与
- 参照: `docs/ja/develop/requirements/accessibility.md`

### 5.2 テスト

- [ ] ViewModel の単体テスト拡充（現在は Adapter と展開状態のみ）
- [ ] 操作の結合テストにケース追加（新規 UI ごとに必須）
- [ ] ブレークポイント境界のスクリーンショット比較の仕組み
- [ ] **ヒットテストの自動検証手段**
      `send_mouse_click` は `i-slint-backend-testing` の `internal` フィーチャ配下で、
      公開版はビルド不能（Slint リポジトリ内のパスを `include_dir!` している）。
      上流の修正待ち、または別手段の検討
- 参照: `docs/ja/develop/design/testing.md` の「操作の結合テスト」

### 5.3 パス・設定の受け渡し

- [ ] `UnifiedConfig` にディレクトリを渡せるようにする
      現在は `flequit-app` が起動時に `FLEQUIT_DB_PATH` /
      `FLEQUIT_AUTOMERGE_PATH` を設定する回避策
      （`crates/flequit-app/src/lib.rs` の `publish_storage_paths`）
- [ ] `flequit-settings` の設定ディレクトリを `flequit-platform::paths` 起点にする
      （現在 `HOME` 依存。テストが環境変数を書き換えており直列化が必要）
- [ ] `crates/flequit-repository/src/utils/path_service.rs` の削除
      使われていない重複コード。`#[deprecated]` 済みだが未削除

### 5.4 ロギング

- [ ] ファイル出力（`platform.paths().log_dir()` へローテーション）
- [ ] Android は logcat、iOS は OSLog へ切り替え

### 5.5 ドキュメント

- [ ] `docs/en/` の作成（`docs/ja/` が固まってから）
- [ ] 未実装として注記したパスの解消
  - `crates/flequit-ui/src/adapters/patch.rs`
  - `crates/flequit-ui/src/viewmodels/settings/datetime_format.rs`
  - `crates/flequit-ui/src/viewmodels/task/mod.rs`
  - `crates/flequit-ui/src/viewmodels/user_preferences/`
  - `crates/flequit-ui/ui/globals/settings.slint`

### 5.6 パフォーマンス

- [ ] 変更のたびにプロジェクト全体を再読込している点の見直し
      （`reload_projects`。正確さを優先した暫定実装）
- [ ] 大量タスクでの `ListView` 検証
- [ ] 起動時の初期クエリ件数上限

---

## 6. P3 — Phase 2 モバイル

### 6.1 プラットフォーム実装

- [ ] Android: サンドボックスルート取得（`android-activity`）
- [ ] Android: 通知（チャネル + 実行時許可）
- [ ] Android: SAF によるファイル選択（`FileHandle::Opaque`）
- [ ] Android: `Intent.ACTION_VIEW`
- [ ] iOS: サンドボックスルート取得
- [ ] iOS: `UNUserNotificationCenter`
- [ ] iOS: `UIDocumentPickerViewController`
- [ ] iOS: `UIApplication.open`
- [ ] ライフサイクル購読（`Suspend` / `Resume` / `LowMemory`）の実装と接続

### 6.2 ビルド基盤

- [ ] `mobile/android/`（マニフェスト、アイコン、xbuild 設定）
- [ ] `mobile/ios/`（XcodeGen 設定、Info.plist）
- [ ] `crates/flequit-app/src/mobile.rs`（`android_main` / iOS エントリ）
- [ ] SQLite のクロスコンパイル確認（`libsqlite3-sys` の `bundled`）
- [ ] CI のモバイルジョブを `continue-on-error` から外す

### 6.3 実機検証

- [ ] セーフエリア、慣性スクロール、ソフトキーボード
- [ ] 長押しメニュー
- [ ] バックグラウンド遷移時のデータ保全

---

## 7. P4 — 配布・将来

- [ ] `cargo-packager` の設定と各 OS インストーラ生成
- [ ] コード署名（Windows / macOS notarization / Android / iOS）
- [ ] `cargo audit` の CI 組み込み
- [ ] 自動アップデート（`Capability::SelfUpdate`）
- [ ] システムトレイ / グローバルショートカット（デスクトップのみ）
- [ ] Automerge によるクラウド同期（roadmap ver1.2 以降）
- [ ] Web 版（同期サーバ設計の確定後）

---

## 8. 環境メモ

- Linux 実行には `libxkbcommon-x11-0` が必要
- テスト前に `./scripts/test-prepare.sh`（SQLite テンプレート DB を 1 度だけ作成）
- 本リポジトリはまだ Git リポジトリではない。`git init` が未実行
