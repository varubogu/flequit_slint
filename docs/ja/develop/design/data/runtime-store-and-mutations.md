# Runtime Store と Mutation

アプリ実行中のエンティティ状態と、SQLite + Automerge をまたぐ変更処理の
一貫性を定める。

- 決定日: 2026-09-12
- 状態: 方針確定 / 実装着手
- 上位の記録: `plans/plan.md` 8.

## 正となる状態

Desktop / Mobile の永続化ではローカル（SQLite + Automerge）が正である。
アプリ実行中は Rust の `Runtime Store` を唯一の状態源とし、Slint の
`AppState` と各 `Model` は Store から生成する表示用 projection とする。

```text
Slint View
    ↓ Action
ViewModel → Runtime Store → UI projection
    ↓ facade
flequit-core
    ↓ transaction port
flequit-infrastructure
    ├─ SQLite
    └─ Automerge
```

Slint 型は `Send` ではないため、Runtime Store はドメイン型だけを保持する。
Slint の `Model` はUIスレッド上でViewModelが所有し、Storeの変更通知を
`upgrade_in_event_loop` 経由で反映する。

## Revision

- 基本単位はエンティティ単位の単調増加 `revision` とする。
- 並び替え、親子移動、カスケード削除など複数エンティティにまたがる操作は
  プロジェクト単位の revision も使用する。
- revision はSQLiteへ永続化し、楽観的更新の競合判定に利用する。
- Automerge の heads とアプリケーション revision は別物として扱う。

## Pending Mutation

ユーザー操作は再現可能な `Mutation` としてStoreへ登録する。

| フィールド | 内容 |
| --- | --- |
| `operation_id` | 冪等性と追跡に使う一意なID |
| `target` | エンティティ種別、プロジェクトID、エンティティID |
| `base_revision` | Mutationを作成した時点の確定revision |
| `forward_patch` | 適用する変更 |
| `rollback_data` | 変更したフィールドの直前値、または補償に必要な値 |
| `status` | `pending` / `committed` / `failed` |

StoreはエンティティごとにMutationを直列化する。表示上は複数のPending Mutationを
重ねてよいが、永続化は登録順に行う。

失敗したMutationより新しいrevisionが無ければ逆差分を適用できる。新しいMutationが
存在する場合は逆差分を現在値へ直接適用せず、最後の確定状態から失敗したMutationを
除外し、残りのPending Mutationを順番に再適用して表示状態を再計算する。

## 永続操作ジャーナル

RAM上のPending Mutationとは別に、クラッシュ復旧用の操作ジャーナルをSQLiteへ
永続化する。一時ディレクトリのファイルはOSに削除される可能性があるため使用しない。

| フィールド | 内容 |
| --- | --- |
| `operation_id` | Runtime StoreのMutationと同じID |
| `target` | 対象種別とID |
| `base_revision` | 操作開始時のrevision |
| `forward_data` | 未完了側への再適用に必要なデータ |
| `rollback_data` | 適用済み側の補償に必要なデータ |
| `status` | `prepared` / `committed` / `failed` |
| `created_at` | 復旧順序と監査用の日時 |

AutomergeスナップショットがSQLiteへ保持するには大きすぎる場合だけ、アプリの
データディレクトリに操作IDを名前に含むリカバリーファイルを置く。SQLiteの
ジャーナルはそのパスとチェックサムを保持し、孤立ファイルを起動時に回収できる
ようにする。

## 保存手順

ローカル保存はSQLiteとAutomergeの両方が成功した時点で確定する。

1. 操作ジャーナルへ `prepared` を記録して確定する
2. SQLite transactionを開始する
3. SQLiteへ変更とrevision更新を適用する
4. Automergeへ同じ意味の変更を適用する
5. SQLite transactionをcommitする
6. 操作ジャーナルを `committed` にする
7. Runtime StoreのMutationを確定する

既知の失敗では次のように戻す。

- SQLite変更失敗: SQLite transactionをrollbackする
- Automerge変更失敗: SQLiteをrollbackし、必要ならAutomergeを補償する
- SQLite commit失敗: Automergeをスナップショット復元または補償Mutationで戻す
- Store: 失敗したMutationを除外して表示状態を再計算する

## クラッシュ復旧

起動時に `prepared` の操作を検出し、次の順に処理する。

1. SQLiteとAutomergeへの適用状態を調査する
2. 未適用側への冪等な再適用を試す
3. 両方が揃えば `committed` にする
4. 再適用できなければ適用済み側へ補償Mutationを適用して `failed` にする
5. 復旧後のSQLiteからRuntime Storeを構築する

同期開始前はAutomergeスナップショット復元を使用できる。外部との同期開始後は
取り込んだ変更を消さないよう、履歴を巻き戻さず補償Mutationを追加する。

## 境界と責務

- View / `.slint`: 表示と入力のみ
- ViewModel: Store操作、UI projection、facade呼び出し、UIエラー表示
- Runtime Store: 確定状態、revision、Pending Mutation、再計算、変更通知
- facade: ユーザー操作単位の調整とトランザクション境界
- infrastructure: SQLite transaction、操作ジャーナル、Automerge復元・補償

ViewModelとRuntime StoreはSQLite、Automerge、repository traitを参照しない。

## テスト要件

- 同一エンティティへの複数Pending Mutationから途中の失敗だけを除外できる
- 異なるフィールドと同一フィールドの連続変更を再計算できる
- SQLite失敗時にAutomergeへ変更が残らない
- Automerge失敗時にSQLiteへ変更が残らない
- SQLite commit失敗時にAutomergeが補償される
- `prepared` の各中断地点から起動時復旧できる
- 同じ `operation_id` の再適用が冪等である

## 関連

- [保存先の選択](./storage-targets.md)
- [Automerge-Repo データフロー](./automerge-repo-dataflow.md)
- [ViewModel アーキテクチャ](../ui/viewmodel-architecture.md)
- [UI とコアの接続](../ui/core-bridge.md)
