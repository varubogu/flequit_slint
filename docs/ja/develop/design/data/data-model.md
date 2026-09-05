# データモデル仕様

## 概要

Flequit アプリケーションで使用されるデータモデルの定義と型変換規則を定義します。

## 型システムと変換規則

Slint 版では UI とドメインが同一言語（Rust）のため、
Tauri 版にあった TypeScript との型変換は不要になりました。
代わりに **ドメイン型 → Slint UI 型** の変換が表示層で発生します。

### 型変換表

| Rust 内部型 | Slint UI 型 | SQLite | PostgreSQL | Automerge JSON | 説明 |
| --- | --- | --- | --- | --- | --- |
| `ProjectId` | `string` | `TEXT` | `UUID` | `string` | プロジェクト一意識別子（UUID v4） |
| `AccountId` | 非公開（UI に渡さない） | `TEXT` | `UUID` | `string` | アカウント内部識別子（UUID v4・非公開） |
| `UserId` | `string` | `TEXT` | `UUID` | `string` | ユーザー識別子（UUID v4・公開用） |
| `TaskId` | `string` | `TEXT` | `UUID` | `string` | タスク一意識別子（UUID v4） |
| `TaskListId` | `string` | `TEXT` | `UUID` | `string` | タスクリスト一意識別子（UUID v4） |
| `TagId` | `string` | `TEXT` | `UUID` | `string` | タグ一意識別子（UUID v4） |
| `SubTaskId` | `string` | `TEXT` | `UUID` | `string` | サブタスク一意識別子（UUID v4） |
| `DateTime<Utc>` | `string`（整形済み表示文字列） | `TEXT` | `TIMESTAMPTZ` | `string` | ISO 8601 形式日時文字列 |
| `Option<T>` | `T` + 既定値、または `bool` の有無フラグ | `NULL` | `NULL` | `null` | Optional 値 |
| `String` | `string` | `TEXT` | `TEXT` | `string` | 文字列 |
| `i32` | `int` | `INTEGER` | `INTEGER` | `number` | 32bit 整数 |
| `bool` | `bool` | `INTEGER` | `BOOLEAN` | `boolean` | 真偽値（SQLite は 0/1） |
| Enum 型 | `enum`（Slint 側で同名定義） | `TEXT` | `TEXT` | `string` | 列挙型（文字列として保存） |

### Slint UI 型への変換原則

Slint の `struct` は `Option` を表現できないため、変換時に必ず解決する。

| ドメイン | Slint | 変換方法 |
| --- | --- | --- |
| `Option<DateTime<Utc>>` | `due-label: string` + `has-due: bool` | 未設定は空文字 + `false` |
| `Option<String>` | `string` | 未設定は空文字 |
| `Option<i32>` | `int` + `has-value: bool` | 0 と未設定を区別する必要がある場合 |
| `DateTime<Utc>` | `string` | ユーザーのタイムゾーンとフォーマット設定で整形 |
| `TaskStatus` | `enum TaskStatus` | 同名の enum を `.slint` にも定義 |

変換は `crates/flequit-ui/src/adapters/` に集約し、純粋関数として実装する。
詳細は `design/ui/core-bridge.md` の「UI 型とドメイン型」を参照。

### 注意点

- **UUID 形式**: 全ての ID は `xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx` 形式
- **日時形式**: `YYYY-MM-DDTHH:mm:ss.sssZ` (UTC)
- **SQLite 真偽値**: `true`=1, `false`=0 で保存
- **Optional 値**: 未設定時は `null`/`NULL` で統一
- **ID の UI 表現**: Slint 側では `string`。ViewModel が受け取った時点で
  ドメインの ID 型へパースし、失敗を握りつぶさない

## UTC ポリシー

アプリケーション内の全日時データは以下のポリシーに従います。

### 内部データ（常に UTC）

- **Rust モデル**: chrono クレートの `DateTime<Utc>` を全フィールドで使用
- **SQLite ストレージ**: 全 TIMESTAMP カラムは UTC ISO 8601 文字列
  （`YYYY-MM-DDTHH:mm:ss.sssZ`）として保存
- **Automerge CRDT**: 全日時フィールドは UTC ISO 8601 文字列として保存
- **日付のみの規約**: 日付のみの値は UTC 深夜（`T00:00:00Z`）として保存

Tauri 版にあった「IPC レイヤーでのシリアライズ」は存在しません。
ドメイン層から表示層まで `DateTime<Utc>` のまま流れ、変換は Adapter で 1 回だけ行われます。

### 表示レイヤー（ユーザーのタイムゾーン）

- ユーザーに表示する全日時は、ユーザーの有効タイムゾーンに変換する
- ユーザータイムゾーンの取得元: 設定値 `general.timezone`
  （`"system"` 設定時は OS のタイムゾーンにフォールバック）
- フォーマット関数は `timezone: &Tz` を引数で受け取る（暗黙のグローバル参照をしない）
- 変換は Adapter（`crates/flequit-ui/src/adapters/datetime.rs`）でのみ行う
- `.slint` 側では整形済み文字列を受け取るだけとし、日時計算をしない

### テストのルール

- 全日付リテラルには UTC の意図を明示する: `"2025-01-15T12:00:00Z".parse::<DateTime<Utc>>()`
- フォーマット関数のテストではタイムゾーンを明示的に渡す（既定値に依存しない）
- タイムゾーン依存のテストは `Asia/Tokyo` と `UTC` の両方で検証する

## エンティティ定義

`./entity/*.md` を参照
