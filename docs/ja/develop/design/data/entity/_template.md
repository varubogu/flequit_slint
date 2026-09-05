# エンティティ定義テンプレート

統合エンティティドキュメント (`projects.md` / `settings.md` / `accounts-and-users.md` / `user-preferences.md`) で各エンティティを記述する際の共通フォーマット。

## 各エンティティの記述項目

```markdown
## {EntityName} (日本語名) — {table_name}

**役割**: 1〜2 行の概要

### フィールド

| 論理名 | 物理名 | 型 (Rust) | 制約 | デフォルト | 外部キー | 説明 |
| --- | --- | --- | --- | --- | --- | --- |
| ... | ... | ... | PK/UK/NN | ... | ... | ... |

### 制約

- PRIMARY KEY: ...
- FOREIGN KEY: ...
- NOT NULL: ...
- UNIQUE: ... (該当時)

### インデックス対象カラム

`column_a`, `column_b` (SQL 文は実装側 `crates/flequit-infrastructure-sqlite/...` のマイグレーションを正本とする)

### 関連

- 関連テーブル名: 概要

### 補足 (任意)

- 設計上の特記事項があれば 1〜3 行で
```

## 命名・型ガイド

- ID 型: 専用型 (`ProjectId`, `TaskId` 等)
- 日時型: `DateTime<Utc>` (ISO 8601)
- Optional: `Option<T>` (NULL 許可)
- 数値: `i32`、論理値: `bool`、文字列: `String`
- UI (Slint) 型への変換規則は `../data-model.md` の「Slint UI 型への変換原則」を参照

詳細な型変換規則は `../data-model.md` を参照。

## SQL/コード例の方針

- **記載しない**。SQL DDL、Sea-ORM コード、Slint の `struct` 定義、Rust サービスコードはこのドキュメント群に置かない。
- 実装は `crates/flequit-infrastructure-sqlite/` 等の正本を参照する。
