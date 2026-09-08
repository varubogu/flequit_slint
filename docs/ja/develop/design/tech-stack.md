# 技術スタック・プロジェクト構造

## 対応プラットフォーム

Flequit (Slint 版) は **Rust 単一プロセス** のネイティブアプリケーションである。
WebView / IPC を持たず、UI からドメインロジックまでを 1 つのバイナリで完結させる。

| プラットフォーム | 対応 | 実装フェーズ | ビルド手段 |
| --- | --- | --- | --- |
| Windows | 必須 | Phase 1 | `cargo build` + `cargo-packager` |
| macOS | 必須 | Phase 1 | `cargo build` + `cargo-packager` |
| Linux | 必須 | Phase 1 | `cargo build` + `cargo-packager` |
| Android | 必須 | Phase 2 | Gradle + `cargo-ndk`（`mobile/android/`） |
| iOS | 必須 | Phase 2 | XcodeGen + `cargo build --target aarch64-apple-ios`（`mobile/ios/`） |
| Web (WASM) | UI のみ | Phase 2 | `wasm-bindgen`（`crates/flequit-web` + `web/`） |

- **Phase 1 / Phase 2** は実装着手順であり、設計・ドキュメントは最初から全対応で記述する。
- Web ビルドは **UI シェルの描画のみ**。`sea-orm` / `sqlx-sqlite` は wasm32 で動かず、
  かつブラウザ内に永続化層を作る予定も無い（保存はバックエンドサーバが担う）。
  そのため `crates/flequit-web` は `flequit-ui` に依存せず、同じ `.slint` を
  再コンパイルする構成にしている。詳細は `plans/plan.md` 7. / 8.
- Slint の iOS サポートは **Rust のみ**。本プロジェクトは Rust 単一言語のため制約にならない。
- UI は単一コードベースとし、画面幅のブレークポイントでレイアウトを切り替える
  （`design/ui/responsive-layout.md` 参照）。

## 技術スタック

### UI

- **フレームワーク**: Slint 1.x（`.slint` 宣言的 UI 言語 + Rust バインディング）
- **レンダラ**: FemtoVG（デスクトップ既定）/ Skia（モバイル・高負荷描画時の選択肢）
- **ウィンドウ/バックエンド**: winit（デスクトップ）/ `android-activity`（Android）/ UIKit（iOS）
- **i18n**: Slint 組み込み翻訳（`@tr()` + bundled translations）
- **アイコン**: SVG アセット（`crates/flequit-ui/ui/assets/icons/`）
- **テーマ**: Slint global singleton によるトークン定義（ライト/ダーク/カスタム）

### コア / データ

- **言語**: Rust（edition 2024）
- **データベース**: SQLite（Sea-ORM、local-first）
  - モバイルでは `libsqlite3-sys` の `bundled` feature でクロスコンパイルする
- **CRDT**: Automerge（pure Rust のため全プラットフォームで同一コード）
- **非同期ランタイム**: Tokio
- **ロギング**: `tracing` + `tracing-subscriber`
  - Android は `tracing-android`、iOS は OSLog へ出力先を切り替える

### プラットフォーム連携

OS 連携 API はプラットフォームごとに可否と実装が異なるため、
**`flequit-platform` クレートで抽象化する**。UI やコアから OS API を直接呼ばない。

| 機能 | デスクトップ実装 | Android | iOS |
| --- | --- | --- | --- |
| データ/設定ディレクトリ解決 | `directories` | `android-activity` の内部ストレージパス | `NSDocumentDirectory` |
| ファイル選択 | `rfd` | SAF（Storage Access Framework）経由 | UIDocumentPicker 経由 |
| 通知 | `notify-rust` | 通知チャネル API | `UNUserNotificationCenter` |
| システムトレイ | `tray-icon` | 非対応（機能自体を無効化） | 非対応（同左） |
| URL を既定アプリで開く | `opener` | Intent | `UIApplication.open` |
| 削除ファイルの退避 | `trash`（OS ゴミ箱） | アプリ内 `.deleted/` のみ | アプリ内 `.deleted/` のみ |

未対応機能は `Capability` として実行時に問い合わせ、UI 側で該当項目を非表示にする。
詳細は `design/platform/platform-abstraction.md` を参照。

### ツールチェーン

- **パッケージマネージャ**: Cargo のみ（Node.js / Bun への依存なし）
- **型チェック**: `cargo check --quiet`
- **Lint**: `cargo clippy`
- **Format**: `cargo fmt --all`
- **テスト**: `cargo test -j 4`
- **デスクトップ配布パッケージ**: `cargo-packager`
- **Android ビルド**: `xbuild`
- **iOS ビルド**: XcodeGen + Xcode

## Rust クレート構成

`crates/` 配下の workspace クレート:

| クレート | 責務 |
| --- | --- |
| `flequit-types` | ID 型・enum 等の基本型 |
| `flequit-platform` | プラットフォーム抽象化（パス解決・通知・ダイアログ・Capability 判定） |
| `flequit-model` | ドメインモデル構造体 |
| `flequit-repository` | Repository トレイト（契約） |
| `flequit-core` | ドメインロジック（facade / service） |
| `flequit-infrastructure-sqlite` | SQLite 永続化実装 |
| `flequit-infrastructure-automerge` | Automerge 永続化実装 |
| `flequit-infrastructure` | 複数インフラを合成する統合 Facade |
| `flequit-settings` | 設定ファイルの読み書き |
| `flequit-testing` | テスト用ヘルパ |
| `flequit-ui` | Slint UI 定義 + ViewModel 層 |
| `flequit-app` | 実行バイナリ（初期化・DI・イベントループ・各プラットフォームのエントリポイント） |

依存方向ルール:

```text
flequit-types
  ├→ flequit-platform ─────────────────────┐
  └→ flequit-model                         │
       → flequit-repository                │
         → flequit-core                    │
           → flequit-infrastructure-*  ←────┤
             → flequit-infrastructure       │
               → flequit-ui ←───────────────┘
                 → flequit-app
```

- `flequit-platform` は `flequit-types` のみに依存する葉クレート。
  `flequit-infrastructure-*`（パス解決）と `flequit-ui`（ダイアログ・通知）から参照される。
- 逆方向の依存は禁止。`cargo tree` と CI の依存チェック（`scripts/check-crate-deps.sh`）で検証する。

## プロジェクト構造

```text
(root)
├── Cargo.toml                       # workspace 定義
├── crates/
│   ├── flequit-types/
│   ├── flequit-platform/
│   │   └── src/
│   │       ├── capability.rs        # 実行時 Capability 判定
│   │       ├── paths.rs             # データ/設定/キャッシュディレクトリ
│   │       ├── notification.rs
│   │       ├── file_dialog.rs
│   │       └── platform/            # desktop / android / ios の各実装
│   ├── flequit-model/
│   ├── flequit-repository/
│   ├── flequit-core/
│   ├── flequit-infrastructure/
│   ├── flequit-infrastructure-sqlite/
│   ├── flequit-infrastructure-automerge/
│   ├── flequit-settings/
│   ├── flequit-testing/
│   ├── flequit-ui/
│   │   ├── build.rs                 # slint-build によるコンパイル + 翻訳バンドル
│   │   ├── ui/                      # .slint ファイル群
│   │   │   ├── main.slint           # ルートウィンドウ
│   │   │   ├── globals/             # テーマ・状態・レイアウト・コールバックの global singleton
│   │   │   ├── components/          # 再利用コンポーネント
│   │   │   ├── views/               # 画面単位（サイドバー / タスク一覧 / 詳細 / 設定）
│   │   │   └── assets/
│   │   └── src/
│   │       ├── viewmodels/          # Slint Model/Property ↔ core の橋渡し
│   │       ├── adapters/            # ドメインモデル ↔ Slint 構造体の変換
│   │       └── lib.rs
│   └── flequit-app/
│       └── src/
│           ├── main.rs              # デスクトップのエントリポイント
│           ├── lib.rs               # 共通ブートストラップ
│           └── mobile.rs            # android_main / iOS エントリ [Phase 2]
├── i18n/                            # 翻訳 .po ファイル（bundled translations の入力）
├── mobile/                          # [Phase 2] 未作成
│   ├── android/                     # マニフェスト・アイコン・xbuild 設定
│   └── ios/                         # XcodeGen 設定・Info.plist
├── tests/                           # workspace 横断の統合テスト
└── docs/                            # プロジェクトドキュメント
```

## Svelte + Tauri 版からの主な変更点

| 項目 | Svelte + Tauri 版 | Slint 版 |
| --- | --- | --- |
| UI 記述 | `.svelte` + Tailwind | `.slint` + テーマ global |
| プロセス構成 | WebView + Rust の 2 プロセス | Rust 単一プロセス |
| UI ↔ ロジック | Tauri IPC（JSON シリアライズ） | 直接関数呼び出し |
| 状態管理 | Svelte runes ストア | Slint Property / Model + ViewModel |
| レスポンシブ | Tailwind のブレークポイント | Slint の `states` + 幅条件プロパティ |
| i18n | Inlang Paraglide | Slint bundled translations |
| 型共有 | Specta による Rust → TS 生成 | 共有不要（同一言語） |
| OS 連携 | Tauri プラグイン | `flequit-platform` クレート |
| モバイル | Tauri 2 モバイル（WebView） | Slint ネイティブ（Android / iOS） |
| ビルド | Bun + Vite + Tauri CLI | Cargo + xbuild + Xcode |
