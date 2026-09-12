# UI レイヤーアーキテクチャ

Slint 版では UI とドメインが同一プロセスに同居する。IPC のような物理的な境界がないため、
**クレート境界とモジュール境界で層を強制する**。

> 実装の正本は `crates/flequit-ui/` を参照。本書は責務と依存ルールのみを述べる。

## 層とディレクトリの対応

| 層 | 配置 |
| --- | --- |
| View | `crates/flequit-ui/ui/**/*.slint` |
| Adapter | `crates/flequit-ui/src/adapters/` |
| ViewModel | `crates/flequit-ui/src/viewmodels/` |
| Facade 以降 | `crates/flequit-core/src/facades/` |

完全なディレクトリツリーは
[`../../rules/file-structure.md`](../../rules/file-structure.md) を参照。

## 各層の責務

### View 層（`ui/**/*.slint`）

- レイアウト・見た目・入力受付のみ
- 状態は `property` として宣言し、値は Rust 側から注入される
- 操作は `callback` として宣言し、実装は Rust 側が登録する
- 派生表示（例: 完了率のラベル）は Slint の宣言的バインディングで表現する
- **レスポンシブ判定は View の責務**（`design/ui/responsive-layout.md` 参照）

禁止事項:

- ❌ ドメイン型（`flequit-model` の構造体）を直接扱う
- ❌ `.slint` 内に業務ルールを書く（「期限切れ判定」等は ViewModel 側）
- ❌ 1 ファイルに複数画面を詰め込む
- ❌ 端末種別を直接分岐する（`if is_android` のような記述）。幅で分岐する

### Adapter 層（`src/adapters/`）

- ドメインモデル ↔ Slint 生成構造体の相互変換のみ
- 純粋関数として実装し、副作用・I/O を持たない
- 日時フォーマット・ステータス表示名などの表示用整形もここで行う

依存ルール:

- ✅ `flequit-model` / `flequit-types` / 生成された Slint 型を参照
- ❌ `flequit-core` / `flequit-infrastructure` を参照しない
- ❌ ViewModel を参照しない

### ViewModel 層（`src/viewmodels/`）

- Slint の `callback` 実装を登録する唯一の場所
- UI 状態（選択中タスク、展開状態、フィルタ等）の保持
- 楽観的更新とロールバック
- 非同期処理の起動と、`upgrade_in_event_loop` による UI への反映
- OS 連携（通知・ファイル選択）は `flequit-platform` 経由で呼び出す

依存ルール:

- ✅ `flequit-core`（facade）を呼び出す
- ✅ `flequit-infrastructure`（統合 Facade）を初期化時に受け取る
- ✅ `flequit-platform` を使用する
- ✅ Adapter を使用する
- ❌ `flequit-infrastructure-sqlite` / `-automerge` の具象実装を直接参照しない
- ❌ `flequit-repository` のトレイトを直接呼び出さない（必ず facade 経由）
- ❌ `#[cfg(target_os = ...)]` を書かない（プラットフォーム分岐は `flequit-platform` の中だけ）
- ⚠️ ViewModel 同士の参照は一方向のみ。循環参照は禁止

### Facade 層以降

`design/backend/rust-guidelines.md` を参照。

## 依存関係マトリックス

| From → To | View (.slint) | Adapter | ViewModel | flequit-core | flequit-infrastructure | flequit-infrastructure-* | flequit-platform | flequit-model/types |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| View (.slint) | ⚠️ import のみ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Adapter | ✅ 生成型 | ⚠️ 同位層 | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ |
| ViewModel | ✅ 生成型 | ✅ | ⚠️ 同位層 | ✅ | ✅ | ❌ | ✅ | ✅ |
| flequit-core | ❌ | ❌ | ❌ | ⚠️ 同位層 | ❌ | ❌ | ❌ | ✅ |

凡例: ✅ 推奨 / ⚠️ 同位層は循環依存に注意 / ❌ 違反

## データフロー

### 読み取り（起動時・フィルタ変更時）

```text
ViewModel::load()
    ↓ facade 呼び出し（async）
flequit-core::facades
    ↓
Repository (SQLite)
    ↓ Vec<Task>
Adapter::to_ui_tasks()
    ↓ upgrade_in_event_loop
Slint Model (VecModel<TaskItem>) を更新
    ↓ 宣言的バインディング
View が再描画
```

### 書き込み（ユーザー操作）

```text
View の callback 発火
    ↓
ViewModel のハンドラ
    ├→ 1. 現在の UI 状態をスナップショット
    ├→ 2. Slint Model / property を即時更新（楽観的更新）
    ├→ 3. tokio::spawn で facade を呼び出し永続化
    └→ 4. 失敗時: upgrade_in_event_loop でスナップショットを復元 + エラー表示
```

## 境界の強制方法

IPC がない分、以下の 3 段で境界を守る。

1. **クレート依存**: `flequit-ui/Cargo.toml` に `flequit-infrastructure-sqlite` /
   `flequit-infrastructure-automerge` / `flequit-repository` を書かない。
   書かなければコンパイル時に参照できない
2. **モジュール可視性**: `adapters` は `pub(crate)`、`viewmodels` の内部状態は非公開にする
3. **CI チェック**: `cargo tree -p flequit-ui` の出力から禁止クレートへの直接依存を検出し、
   `flequit-ui` / `flequit-core` 配下に `cfg(target_os)` が無いことを確認する
   （`scripts/check-crate-deps.sh`）

## Svelte 版からの対応関係

| Svelte 版 | Slint 版 | 備考 |
| --- | --- | --- |
| `components/*.svelte` | `ui/views/`, `ui/components/` | 表示のみに徹する点は同じ |
| `stores/*.svelte.ts` | Slint `global` の `property` + ViewModel | 状態の置き場が UI 側に移る |
| `services/ui/` | ViewModel | Svelte 版で廃止予定だった層はここに統合 |
| `services/domain/` | `flequit-core` の facade / service | Rust 側に吸収 |
| `services/composite/` | `flequit-core` の facade | 同上 |
| `infrastructure/backends/tauri/` | 廃止（直接関数呼び出し） | シリアライズ境界が消える |
| `infrastructure/backends/web/` | 移植しない | `architecture.md` §3 参照 |
| Tailwind のブレークポイント | `ui/globals/layout.slint` | レスポンシブの実現手段が変わる |
| Tauri プラグイン | `flequit-platform` | OS 連携の抽象化先 |

## まとめ: 設計原則

1. **View はロジックを持たない**
2. **ViewModel だけが facade を呼ぶ**
3. **Adapter は純粋関数**
4. **UI は具象インフラを知らない**
5. **UI は OS API を直接呼ばない**
6. **UI スレッドをブロックしない**
7. **境界はクレート依存で物理的に強制する**

## 関連ドキュメント

- [ViewModel アーキテクチャ](./viewmodel-architecture.md)
- [Slint 設計パターン](./slint-patterns.md)
- [レスポンシブレイアウト](./responsive-layout.md)
- [コアとの接続](./core-bridge.md)
- [プラットフォーム抽象化](../platform/platform-abstraction.md)
- [全体アーキテクチャ](../architecture.md)
