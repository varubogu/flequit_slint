# エラー表示設計

エラー **型** の設計（階層化されたエラー型、`thiserror`、層ごとの責務）は
[`backend/rust-guidelines.md`](./backend/rust-guidelines.md) の「エラーハンドリング」が正本。
ログの出力先・ローテーション設計は [`../rules/backend.md`](../rules/backend.md) の
「ロギング」が正本。本書は **UI へどう見せるか** のみを扱う。

## 表示手段の使い分け

| 手段 | 用途 | 例 |
| --- | --- | --- |
| インライン表示 | 入力値の検証エラー | 必須項目の未入力、日付の形式不正 |
| トースト通知 | 回復可能・操作を止めない失敗 | 保存失敗後のロールバック、同期の一時失敗 |
| モーダル | ユーザーの判断が必要な失敗 | 競合の解決、未保存変更の破棄確認 |
| 全画面表示 | 続行不能な失敗 | データディレクトリに書き込めない、DB が壊れている |

## ユーザー向けメッセージの原則

- 表示言語は UI 言語に従う。文言は `.slint` 側の `@tr()` で解決する
  （Rust 側で文言を組み立てない。詳細は [`ui/i18n-system.md`](./ui/i18n-system.md)）
- **何が起きたか** と **次に何をすればよいか** を書く
- 内部 ID・ファイルパス・スタックトレースをそのまま表示しない
  （内部 ID の扱いは [`data/data-security.md`](./data/data-security.md) を参照）
- サポート照会用のエラーコードは表示してよい

## ViewModel での分類

`ServiceError` は表示直前に `UiError` へ変換し、次のカテゴリへ分類する。

| カテゴリ | 元のエラー | 表示 |
| --- | --- | --- |
| 対象が見つからない | `NotFound` / `UserNotFound` | トースト + 一覧の再読込 |
| 入力の検証エラー | `ValidationError` / `InvalidArgument` / 制約違反 | インライン表示 |
| ストレージ障害 | その他の `RepositoryError` | モーダル。再試行の導線を出す |
| プラットフォーム非対応 | `PlatformError::Unsupported` | **表示しない**。`Capability` で事前に UI から項目を消す |
| ユーザーによる取り消し | `PlatformError::Cancelled` | **表示しない** |

`Unsupported` と `Cancelled` をエラーとして見せないことが重要。
前者は設計の漏れ（`Capability` 判定の欠落）を示すサインであり、
後者はユーザーの意図した操作だからである。

## 自動リカバリー

- 楽観的更新の永続化に失敗した場合、スナップショットから復元する
  （[`ui/viewmodel-architecture.md`](./ui/viewmodel-architecture.md) 参照）
- 同期の一時的な失敗は指数バックオフで再試行する。
  ローカル操作は成功しているため、ユーザーの操作は止めない
