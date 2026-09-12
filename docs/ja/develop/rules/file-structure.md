# ファイル構成・プロジェクト構造

ディレクトリツリーの **正本は本書**。他のドキュメントは本書へリンクする
（`design/tech-stack.md` はクレートの責務一覧のみを持つ）。

```text
(root)
├── Cargo.toml                       # workspace 定義
├── rust-toolchain.toml
├── crates/
│   ├── flequit-types/               # ID 型・enum 等の基本型
│   ├── flequit-platform/            # プラットフォーム抽象化
│   │   └── src/
│   │       ├── capability.rs        # 実行時 Capability 判定
│   │       ├── paths.rs             # データ/設定/キャッシュ/ログ ディレクトリ
│   │       ├── notification.rs      # ローカル通知
│   │       ├── file_dialog.rs       # ファイル選択
│   │       ├── opener.rs            # 外部アプリ起動
│   │       ├── lifecycle.rs         # サスペンド/レジューム
│   │       ├── error.rs
│   │       └── platform/            # desktop / android / ios（cfg はここだけ）
│   ├── flequit-model/               # ドメインモデル構造体
│   ├── flequit-repository/          # Repository トレイト（契約）
│   │   ├── src/repositories/
│   │   └── tests/
│   ├── flequit-core/                # ドメインロジック
│   │   └── src/
│   │       ├── facades/             # トランザクション境界・複数 service の協調
│   │       ├── services/            # ドメインロジック（ストレージ非依存）
│   │       └── errors/
│   ├── flequit-infrastructure-sqlite/
│   │   ├── src/
│   │   │   ├── models/              # Sea-ORM エンティティ
│   │   │   ├── migrator/
│   │   │   ├── repositories/
│   │   │   ├── infrastructure/      # DatabaseManager 等
│   │   │   ├── core_ports_impls.rs
│   │   │   ├── testing/
│   │   │   └── bin/                 # migration_runner
│   │   └── tests/
│   ├── flequit-infrastructure-automerge/
│   │   ├── src/
│   │   │   ├── models/
│   │   │   ├── infrastructure/      # DocumentManager、FileStorage、.deleted/ 管理
│   │   │   └── core_ports_impls.rs
│   │   └── tests/
│   ├── flequit-infrastructure/      # 複数インフラを合成する統合 Facade
│   ├── flequit-settings/            # 設定ファイルの読み書き
│   │   └── tests/
│   ├── flequit-testing/             # テスト用ヘルパ（データビルダー）
│   ├── flequit-ui/                  # Slint UI + ViewModel
│   │   ├── build.rs                 # slint-build（コンパイル + 翻訳バンドル）
│   │   ├── ui/                      # .slint ファイル群
│   │   │   ├── main.slint           # ルートウィンドウ
│   │   │   ├── globals/             # theme / layout / app-state / capabilities /
│   │   │   │                        #   actions / i18n / settings / types
│   │   │   ├── components/          # 再利用コンポーネント
│   │   │   ├── views/               # 画面単位
│   │   │   │   ├── sidebar/
│   │   │   │   ├── task-list/
│   │   │   │   ├── task-detail/
│   │   │   │   ├── tags/
│   │   │   │   └── settings/
│   │   │   └── assets/              # アイコン・フォント
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── bindings.rs          # slint::include_modules! の再エクスポート
│   │   │   ├── error.rs             # UiError
│   │   │   ├── adapters/            # ドメイン型 ↔ Slint 型（純粋関数）
│   │   │   └── viewmodels/          # UI 状態とコールバック実装
│   │   │       ├── app.rs           # 全体の初期化・配線
│   │   │       ├── task_list_ui.rs  # 展開状態などのビュー状態
│   │   │       ├── reload_gate.rs
│   │   │       ├── ordering/ recurrence/ search/ settings/
│   │   │       └── project_editor.rs, tag_editor.rs
│   │   └── tests/
│   │       └── interaction.rs       # 操作の結合テスト（UI シェル）
│   ├── flequit-web/                 # wasm32 向け UI プレビュー（永続化なし）
│   │   ├── build.rs                 # .po を flequit-web ドメインへステージング
│   │   └── src/lib.rs
│   └── flequit-app/                 # 実行バイナリ
│       └── src/
│           ├── main.rs              # デスクトップのエントリポイント
│           ├── lib.rs               # 共通ブートストラップ・init_logging
│           ├── entry_android.rs     # android_main（feature = "android"）
│           └── entry_ios.rs         # iOS エントリ（feature = "ios"）
├── i18n/                            # 翻訳ファイル
│   ├── flequit-ui.pot
│   ├── en/LC_MESSAGES/flequit-ui.po
│   └── ja/LC_MESSAGES/flequit-ui.po
├── mobile/
│   ├── android/                     # Gradle + cargo-ndk
│   └── ios/                         # XcodeGen 設定・Info.plist
├── web/                             # wasm の配信ディレクトリ（index.html + pkg/）
├── scripts/
│   ├── check-crate-deps.sh          # 依存方向・cfg 隔離の検証
│   ├── test-prepare.sh              # テスト用 SQLite DB / ディレクトリ準備
│   └── sync-agent-skills.sh         # エージェント設定の同期
├── plans/                           # 移植計画・設計判断の記録
└── docs/                            # プロジェクトドキュメント
```

**統合テストはルート直下ではなく、各クレートの `tests/` に置く**
（ルートに `tests/` は存在しない）。

## 配置ルール

| 内容 | 配置先 |
| --- | --- |
| 画面全体 | `crates/flequit-ui/ui/views/<screen>/` |
| 複数画面で使う UI 部品 | `crates/flequit-ui/ui/components/` |
| アプリ全体で共有する状態・トークン | `crates/flequit-ui/ui/globals/` |
| ドメイン型 → UI 型の変換 | `crates/flequit-ui/src/adapters/` |
| UI 状態とコールバック実装 | `crates/flequit-ui/src/viewmodels/` |
| 画面 1 操作に対応する API | `crates/flequit-core/src/facades/` |
| ドメインロジック | `crates/flequit-core/src/services/` |
| OS 依存処理 | `crates/flequit-platform/src/platform/` |
| クレート内の単体テスト | 同一ソース内の `#[cfg(test)] mod tests` |
| クレート内の結合テスト | `crates/<crate>/tests/` |
| UI 操作の結合テスト | `crates/flequit-ui/tests/interaction.rs` |

## 禁止事項

- `crates/flequit-ui/ui/` にドメインロジックを置く
- `crates/flequit-ui` 以外に `.slint` を置く
- `flequit-platform` 以外に `#[cfg(target_os = ...)]` を書く
- `crates/flequit-ui/src/adapters/` に I/O を持ち込む
