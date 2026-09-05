# ファイル構成・プロジェクト構造

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
│   │   └── src/repositories/
│   ├── flequit-core/                # ドメインロジック
│   │   └── src/
│   │       ├── facades/             # トランザクション境界・複数 service の協調
│   │       ├── services/            # ドメインロジック（ストレージ非依存）
│   │       └── errors/
│   ├── flequit-infrastructure-sqlite/
│   │   └── src/
│   │       ├── models/              # Sea-ORM エンティティ
│   │       ├── migrations/
│   │       ├── repositories/
│   │       └── infrastructure/      # DatabaseManager 等
│   ├── flequit-infrastructure-automerge/
│   │   └── src/
│   │       ├── document_manager.rs
│   │       ├── storage/             # FileStorage、.deleted/ 管理
│   │       └── repositories/
│   ├── flequit-infrastructure/      # 複数インフラを合成する統合 Facade
│   ├── flequit-settings/            # 設定ファイルの読み書き
│   ├── flequit-testing/             # テスト用ヘルパ
│   ├── flequit-ui/                  # Slint UI + ViewModel
│   │   ├── build.rs                 # slint-build（コンパイル + 翻訳バンドル）
│   │   ├── ui/                      # .slint ファイル群
│   │   │   ├── main.slint           # ルートウィンドウ
│   │   │   ├── globals/             # Theme / Layout / AppState / Capabilities / Actions / I18n
│   │   │   ├── components/          # 再利用コンポーネント
│   │   │   ├── views/               # 画面単位
│   │   │   │   ├── sidebar/
│   │   │   │   ├── task-list/
│   │   │   │   ├── task-detail/
│   │   │   │   └── settings/
│   │   │   └── assets/              # アイコン・フォント
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── bindings.rs          # slint::include_modules! の再エクスポート
│   │       ├── adapters/            # ドメイン型 ↔ Slint 型（純粋関数）
│   │       └── viewmodels/          # UI 状態とコールバック実装
│   └── flequit-app/                 # 実行バイナリ
│       └── src/
│           ├── main.rs              # デスクトップのエントリポイント
│           ├── lib.rs               # 共通ブートストラップ
│           └── mobile.rs            # android_main / iOS エントリ [Phase 2]
├── i18n/                            # 翻訳ファイル
│   ├── flequit-ui.pot
│   ├── en/LC_MESSAGES/flequit-ui.po
│   └── ja/LC_MESSAGES/flequit-ui.po
├── mobile/                          # [Phase 2] 未作成
│   ├── android/                     # マニフェスト・アイコン・xbuild 設定
│   └── ios/                         # XcodeGen 設定・Info.plist
├── tests/                           # workspace 横断の統合テスト
│   ├── integration/
│   └── system/
├── scripts/
│   ├── check-crate-deps.sh          # 依存方向・cfg 隔離の検証
│   └── sync-agent-skills.sh         # エージェント設定の同期
└── docs/                            # プロジェクトドキュメント
```

`[Phase 2]` の項目はモバイル対応時に作成する。現時点では存在しない。

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
| クレート横断の統合テスト | `tests/integration/` |

## 禁止事項

- `crates/flequit-ui/ui/` にドメインロジックを置く
- `crates/flequit-ui` 以外に `.slint` を置く
- `flequit-platform` 以外に `#[cfg(target_os = ...)]` を書く
- `crates/flequit-ui/src/adapters/` に I/O を持ち込む
