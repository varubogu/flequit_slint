# UI 固有のコーディングルール

対象: `crates/flequit-ui`（`.slint` ファイルと ViewModel / Adapter）

本書は **レビューで確認する項目** を列挙する。各ルールの根拠と具体例は
リンク先の設計書を参照。

| 領域 | 根拠となる設計書 |
| --- | --- |
| `.slint` の書き方 | [`../design/ui/slint-patterns.md`](../design/ui/slint-patterns.md) |
| ViewModel の設計 | [`../design/ui/viewmodel-architecture.md`](../design/ui/viewmodel-architecture.md) |
| 層の責務と依存 | [`../design/ui/layers.md`](../design/ui/layers.md) |
| レスポンシブ | [`../design/ui/responsive-layout.md`](../design/ui/responsive-layout.md) |
| i18n | [`../design/ui/i18n-system.md`](../design/ui/i18n-system.md) |
| UI ↔ コア | [`../design/ui/core-bridge.md`](../design/ui/core-bridge.md) |

---

## `.slint`

- [ ] ドメイン型を扱っていない（UI 型の `struct` のみ）
- [ ] ロジックが混入していない。検索クエリの解釈、繰り返しの日付計算、
      期限キーワードの判定、永続化を伴う状態変更は **ViewModel の責務**
- [ ] 状態の置き場所が適切
      （Rust が知る必要のない一時状態はローカル `property`。
      安易に `global` を増やさない）
- [ ] プロパティは既定 `in`。`in-out` はテキスト入力の `text` 等、
      双方向が必須の場合のみ
- [ ] 派生値は `private property <T>: <式>;` で宣言的に書いている
      （Rust 側で計算して流し込んでいない）
- [ ] 数十件を超えうるリストは `ListView`（仮想スクロール）を使っている
- [ ] `export component` は外部から使うものだけに付いている
- [ ] 親子間の通知は `callback`。名前は動詞または「動詞-目的語」
- [ ] 子コンテンツの受け渡しは `@children`
- [ ] 200 行を超えたら分割を検討した

## ViewModel

- [ ] Slint の `callback` 登録がここに集約されている
- [ ] クロージャがキャプチャするのは `slint::Weak` のみ（強参照は循環参照）
- [ ] バックグラウンドからの UI 更新は `upgrade_in_event_loop()` 経由
- [ ] 1 件の変更で `set_vec()` を呼んでいない（`set_row_data()` 等の差分更新）
- [ ] `Rc<RefCell<T>>` の `borrow_mut()` スコープが最小（再入で panic する）
- [ ] グローバル変数・`static` によるシングルトンを作っていない
- [ ] Repository / 具象インフラを直接呼んでいない（`flequit-core` の facade 経由）

## Adapter

- [ ] 純粋関数である（I/O・副作用・グローバル参照を持たない）
- [ ] ドメイン型 ↔ Slint 型の変換のみを行っている
- [ ] 日時フォーマットはタイムゾーンを引数で受け取っている

## 層の境界（違反すると CI で落ちる）

- [ ] `flequit-ui` に `#[cfg(target_os = ...)]` を書いていない
- [ ] OS API を直接呼んでいない（`flequit-platform` 経由）
- [ ] `flequit-infrastructure-*` の具象クレートに直接依存していない

```rust
// ❌ NG: 具象インフラへの直接アクセス
use flequit_infrastructure_sqlite::repositories::TaskRepository;

// ✅ OK: facade 経由
use flequit_core::facades::task_facade;
```

## レスポンシブ

- [ ] レイアウト分岐は **画面幅**。端末種別・OS で分岐していない
- [ ] ブレークポイント判定は `global Layout` を参照している
- [ ] ピクセル値をハードコードせず `Theme` / `Layout` のトークンを使っている
- [ ] タップ領域が 48px 以上ある

## アクセシビリティ

対話可能な要素には以下を **必ず** 付ける。省略すると支援技術からも
`tests/interaction.rs` からも到達できない。

- [ ] `accessible-role`
- [ ] `accessible-label`（プレースホルダを使う。単語連結で組み立てない）
- [ ] `accessible-action-default`
- [ ] 状態を持つ要素は該当プロパティも設定
      （`accessible-checked` / `accessible-expanded` / `accessible-item-selected` 等）
- [ ] 新しい対話要素を追加したら `crates/flequit-ui/tests/interaction.rs` に
      ケースを追加した

## 国際化

- [ ] 全ての表示文字列を `@tr()` で記述している
- [ ] Rust 側で文言を組み立てていない（識別子を渡して `.slint` でマップ）
- [ ] 単語連結で文を組み立てていない（語順が言語で変わる）
- [ ] 1 語だけの文言に文脈（`"Context" =>`）を付けた

## 開発ワークフロー

[`workflow.md`](./workflow.md) を参照。
