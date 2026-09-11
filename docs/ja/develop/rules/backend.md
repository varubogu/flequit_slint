# バックエンド固有のコーディングルール

対象: `flequit-core` / `flequit-repository` / `flequit-infrastructure-*` /
`flequit-model` / `flequit-types` / `flequit-platform`

## Rust の書き方

### Option 型の扱い

- Option から値を取り出す際、1 つだけなら `if let Some` で取ってよいが、
  複数ある場合はネストが深くならないように一時的に変数に格納する

### エラー

- 各層で `thiserror` によるエラー型を定義する
- `anyhow` は内部の処理チェーンでのみ使い、公開 API では型付きエラーを返す
- `unwrap()` は「起こり得ない」ことを証明できる箇所のみに限る

## モジュールの関連性

### アーキテクチャ

クリーンアーキテクチャ採用

```text
UI レイヤー（Slint + ViewModel）
↓
ドメイン層（facade が呼び出され、場合によってはそこから複数の service を呼び出す）
↓
データアクセス層（repository、実体としては sqlite や automerge など）
```

### アクセス制御ルール

- **ViewModel**: facade は OK、service, repositories は NG
- **facade**: service は OK、facade 同士、ViewModel、repositories は NG
- **service**: service と repository は OK、ViewModel, facade は NG
- **repository**: repository 内のみ OK、外部は NG
- **flequit-platform**: どの層からも参照可。ドメイン知識を持たない

### クレート依存の方向

**正本は `scripts/check-crate-deps.sh`**（CI で検証される）。
図と依存可能先の一覧は
[`../design/backend/rust-guidelines.md`](../design/backend/rust-guidelines.md) を参照。
逆方向の依存は禁止。

## プラットフォーム分岐

- `#[cfg(target_os = ...)]` / `#[cfg(target_arch = ...)]` は
  **`flequit-platform` の内部にのみ** 書く
- 他クレートで OS 判定が必要になった場合、それは `flequit-platform` に
  抽象化が足りていないサイン
- ファイルパスは必ず `flequit-platform::paths` を起点にする。ハードコード禁止

## 非同期処理

- Tokio ランタイムは `flequit-app` が構築する。下位層でランタイムを作らない
- `flequit-core` 以下は Slint を知らない。UI への通知はチャネルで行う
- CPU 集約処理・同期 I/O は `spawn_blocking` へ委譲する
- 独立した I/O は `tokio::join!` / `try_join!` で並行実行する

## トランザクション

- **facade 層でのみ** トランザクションを開始する（ネスト防止）
- service / repository は受け取ったトランザクションで操作し、自分で commit / rollback しない
- 読み取り専用操作にトランザクションを使わない

詳細は `docs/ja/develop/design/backend/transaction-management.md` を参照。

## ロギング

- `tracing` を使う。`log` クレートは使用しない
- facade の公開 API には `#[tracing::instrument]` を付与する
- ログメッセージは英語固定（翻訳対象外）
- 出力先は `flequit-app::init_logging` が組み立てる。標準エラー出力と、
  `platform.paths().log_dir()` 配下の日次ローテーションファイル
  （`flequit.<日付>.log`、7 世代で自動削除）の 2 系統
- ファイル書き込みは `tracing-appender` のノンブロッキング。返される
  `WorkerGuard` は `run()` が握り続ける。先に drop すると末尾のログが消える
- ログディレクトリを開けない場合はファイル出力のみ諦めて起動を続ける
- 標準エラー出力が読めないプラットフォームでは `flequit-platform::SystemLogWriter`
  が OS 側のログへ流す（Android は `__android_log_write` で logcat、
  wasm32 はブラウザの console）。デスクトップと iOS は stderr がそのまま読めるため
  `SystemLogWriter::current()` は `None` を返す。この選択には `cfg(target_os)` /
  `cfg(target_arch)` が要り、それらは `flequit-platform` にしか置けない

## 開発ワークフロー

詳細は `docs/ja/develop/rules/workflow.md` を参照してください。
