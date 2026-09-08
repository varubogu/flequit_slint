# 部分更新システム実装設計書

Flequit における **フィールド単位の部分更新システム** の設計。`partially` クレートを活用して
更新範囲を最小化し、型安全性を保つ。

> 実装の正本は `crates/flequit-model/`、`crates/flequit-core/src/services/`、
> `crates/flequit-ui/src/viewmodels/` を参照。

## 1. 背景と要件

### 課題

- **更新範囲**: 1 列の変更でも行全体を書き戻すため、不要な DB 更新と Automerge 変更が発生する
- **競合**: Automerge 上で行全体を差し替えると、他端末の別フィールド変更と衝突しやすい
- **保守性**: 列単位の関数を個別作成すると膨大なコード量になる

Slint 版では IPC が無いため「転送量」の課題は消えたが、
**DB 更新範囲と CRDT の競合削減** という本質的な動機は変わらない。

### 要件

- 読み込み: 全データ / テーブル単位 / 行単位 / 列単位
- 書き込み: 全データ (初回のみ) / 行単位 (新規追加) / 列単位 (リアルタイム更新)
- 複数 Repository 実装 (SQLite / Automerge / 将来クラウド・Web)

## 2. 設計方針

1. **Patch / Delta Update パターン** の採用
2. **`partially` クレート** による自動生成
3. **既存システムとの併存** による段階的導入
4. **型安全性** の最大限活用

## 3. 技術選定

| アプローチ | データ転送量 | 実装コスト | 保守性 | 型安全性 | Automerge 親和性 |
| --- | --- | --- | --- | --- | --- |
| **Patch Update (`partially`)** ✓採用 | ⭐⭐⭐ | ⭐⭐⭐⭐ | ⭐⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐ |
| Field Specific Commands | ⭐⭐⭐ | ⭐ | ⭐ | ⭐⭐⭐ | ⭐⭐ |
| Generic Field Update | ⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐ | ⭐ | ⭐⭐ |
| 現状維持 | ⭐ | ⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐ | ⭐⭐⭐ |

### 採用理由 (`partially` クレート)

- 成熟した API + 豊富なドキュメント
- `apply_some()` による部分適用と変更検知
- フィールドレベルの詳細制御 (`#[partially(omit)]` 等)
- 自動生成による開発効率向上

## 4. 実装パターン

### 4.1 構造体定義

`models/...rs` で `#[derive(Partial)]` + `#[partially(derive(Debug, Clone, Serialize, Deserialize, Default))]` を付与する。`id` 等の更新対象外フィールドは `#[partially(omit)]` で除外。これにより `XxxPartial` 構造体が自動生成され、各フィールドが `Option<T>` または `Option<Option<T>>` でラップされる。

実装参照: `crates/flequit-model/src/models/task_projects/task.rs` (該当ファイル)

### 4.2 各層の責務

| 層 | 役割 |
| --- | --- |
| **ViewModel** | UI の編集操作から `XxxPartial` を構築して facade に渡す。楽観的更新も担当 |
| **Facade** | 汎用 `update_xxx(id, patch)` と、利便性向上のための専用関数 (`update_xxx_title(id, title)` 等) を提供。専用関数は内部で `XxxPartial { title: Some(...), ..Default::default() }` を構築 |
| **Service** | `repository.find_by_id(id)` → `apply_some(patch)` → 変更があれば `repository.save()` → `Ok(changed)` のフロー |
| **Repository** | パッチ適用後の完全な構造体を `save()` で保存 (現状)。将来 SQL `UPDATE` 最適化を検討 |

### 4.3 UI 側

- `XxxPartial` はドメイン型をそのまま使う (TypeScript 版のような型の二重定義が不要)
- ViewModel は編集前後の UI 型を比較し、変更フィールドのみを `Partial` に詰める
- ViewModel は操作が変えたフィールドだけを `Some` にした `PartialXxx` を組み立てる

実装: `crates/flequit-ui/src/viewmodels/app.rs`。Slint のコールバック 1 つが
編集対象のフィールドを 1 つ（または関連する数個）だけ変えるため、
`PartialTask { title: Some(..), ..Default::default() }` をその場で組み立てて
facade に渡している。編集前後の UI 型を突き合わせて差分を取る
`adapters/patch.rs` は作っていない。比較する相手（編集前の値）を保持する
必要がなく、どのフィールドが変わったかは操作そのものが知っているため。

## 5. 実装課題と対策

### バリデーション戦略

- フィールドレベルバリデーション
- 既存データとの組み合わせバリデーション
- Service 層でのビジネスルール適用

### パフォーマンス

- タスク／サブタスクのタイトルとメモは UI へ即時反映し、最後の入力から
  500ms 経過した時点で ViewModel が最新値だけを永続化する。同じ項目への入力が
  続いている間は待機中の保存を置き換え、キー入力ごとの SQLite／Automerge
  書き込みを避ける
- 設定画面は変更種別を問わず同じ 500ms の待機時間でまとめ、最後の状態だけを
  設定ストアへ保存する。画面表示への反映は待たせない
- タスクの日付確定、追加、削除などの離散的な操作はデバウンスせず即時保存する
- バッチ更新の検討
- Repository 層での SQL 最適化

### Automerge 統合

- 部分更新は SQLite 側で効率実行
- Automerge 側は従来の `save()` で全体保存
- 同期時の自動整合性確保

## 6. 段階的導入計画

| Phase | 内容 |
| --- | --- |
| 1: 基盤 | `partially` クレート追加、`Task` への `Partial` derive、基本パッチ更新 facade |
| 2: 機能拡張 | 頻用フィールド専用 facade 追加、ViewModel 変更検知、バリデーション強化 |
| 3: 最適化 | パフォーマンス測定・調整、Repository SQL 最適化、Automerge 同期効率化 |
| 4: 他エンティティ展開 | Project / Subtask / Tag 等へ適用、統一パターン確立 |

## 7. テスト戦略

| 種別 | 観点 |
| --- | --- |
| 単体 | パッチ適用ロジック、変更検知、バリデーション |
| 結合 | Facade 層 → Repository 層、Automerge と SQLite の整合性 |
| E2E | UI 操作 → 永続化の完全フロー、リアルタイム更新の動作 |

## 8. 実装チェックリスト

### Phase 1

- [ ] `partially` クレートを `Cargo.toml` に追加
- [ ] Task 構造体への `#[derive(Partial)]` 追加
- [ ] `update_task` facade 実装 (patch 受け取り)
- [ ] Facade 層のパッチ処理ロジック
- [ ] Service 層の `apply_some()` 統合
- [ ] ViewModel での差分抽出ユーティリティ

### Phase 2

- [ ] 頻用フィールドの専用 facade 関数追加
- [ ] ViewModel 変更検知ユーティリティ
- [ ] バリデーションルール強化
- [ ] エラーハンドリング改善

## 9. 効果

- **効率的なデータ更新**: 変更フィールドのみを更新 → DB 書き込みと CRDT 変更を最小化
- **型安全性**: `partially` クレートと Rust 型システムによる安全な部分更新
- **保守性**: マクロ自動生成によるコード重複減
- **段階的導入**: 既存システムを残しつつ徐々に移行
