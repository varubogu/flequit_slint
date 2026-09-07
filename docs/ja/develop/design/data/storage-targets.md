# 保存先の選択

タスクデータをどこに置くかは**ユーザーが選ぶ**。アプリの種別（デスクトップ /
モバイル / Web）が保存先を決めるのではない。本書は保存先の構成と、それを
どの層に差し込むかを定める。API スキーマ・認証方式・同期プロトコルは
本書の範囲外で、別途設計する（[未決事項](#未決事項)）。

- 決定日: 2026-09-08
- 状態: 方針確定 / 実装未着手
- 上位の記録: `plans/plan.md` 8.

## 保存先の種類

| 保存先 | Desktop | Mobile | Web | 実体 |
| --- | --- | --- | --- | --- |
| ローカル | ○ | ○ | × | SQLite（検索）+ Automerge（永続化）。現在の実装 |
| クラウドストレージ | ○ | ○ | × | ユーザー所有の同期フォルダ等に置いた Automerge ドキュメント |
| バックエンドサーバ | ○ | ○ | ○ | Flequit が用意するサーバ |

デスクトップ / モバイルは「ローカル専用」ではない。ローカルに加えて
クラウドストレージやバックエンドサーバへ同期できる。

## 正となる保存先

**ローカルが常に正**とする。クラウドストレージとバックエンドサーバは
**同期先**であり、ローカルの内容の複製を受け取る。

| 実行環境 | 正 | 同期先 |
| --- | --- | --- |
| Desktop / Mobile | ローカル | クラウドストレージ、バックエンドサーバ（0 個以上、併用可） |
| Web | バックエンドサーバ | なし |

この形にする理由:

- **書き込みが必ず成功する**。ネットワークの状態に関わらずローカルへ書けるので、
  オフラインのための書き込みキューや、失敗時のフォールバック経路が要らない
- **Automerge の CRDT がそのまま競合解決になる**。「正のコピーが 1 つあって
  他が追従する」形にすると、どちらを勝たせるかという判断がアプリ側に出てくる。
  複製同士を Automerge でマージすれば、その判断は不要になる
- **保存先の切り替えがデータ移行にならない**。ローカルは常に存在するため、
  切り替えは同期先の追加・削除であって、既存データの移行 / 併存 / 破棄の
  選択を伴わない

Web だけは例外で、ブラウザ内に何も保存しない（`plans/plan.md` 7.）。
したがってサーバが正になり、オフラインでは動かない。

## 層構成

既存の層は変えない。保存先の追加は repository trait の裏側で完結する。

```text
flequit-ui (ViewModel)
    ↓ facade
flequit-core            ← 変更なし
    ↓ repository trait
flequit-repository      ← 変更なし
    ↓
flequit-infrastructure  ← 統合層。ローカル（SQLite + Automerge）
    ├─ flequit-infrastructure-sqlite
    ├─ flequit-infrastructure-automerge
    └─ flequit-infrastructure-remote（新規）
```

差し込み方は実行環境で 2 通りに分かれる。同じクレートを別の使い方をする。

| 実行環境 | `flequit-infrastructure-remote` の役割 |
| --- | --- |
| Desktop / Mobile | **同期層**。ローカルへの書き込み後に Automerge の差分を送受信する。読み取り経路には入らない |
| Web | **Repository 実装**。ローカルが無いので、repository trait の実装そのものになる |

`plans/plan.md` 8. には「サーバ向け Repository 実装」とだけ書いてあるが、
ローカルを正にした結果、デスクトップ / モバイルではサーバは読み取り経路に
入らない。Repository 実装として使うのは Web のみ。

保存先の選択そのもの（どこへ同期するか）は `flequit-settings` に持たせ、
`InfrastructureConfig` と同じく `flequit-app` から注入する。パスの解決は
`flequit-platform::paths` 起点という既存の規則（`plans/plan.md` 5.3）を守る。

## 各保存先の扱い

### ローカル

現在の実装のまま。SQLite が検索、Automerge が永続化を担う
（[`automerge-repo-dataflow.md`](./automerge-repo-dataflow.md)）。

### クラウドストレージ

ユーザーが選んだディレクトリに Automerge ドキュメントを置く。アプリから見ると
ローカルのファイルシステムなので、必要なのは書き出し先のパスと、他端末が
書いた変更の取り込みだけになる。ファイル選択は `flequit-platform` の
ファイルダイアログを通す（モバイルでは SAF / `UIDocumentPicker` の
`FileHandle::Opaque` になる）。

### バックエンドサーバ

Flequit が用意するサーバ。Web の唯一の保存先であり、デスクトップ / モバイルでは
同期先の 1 つ。接続状態と同期状態は UI に出す。バックグラウンド同期は
`Capability::BackgroundSync` に接続する（現状は capability を宣言するだけで
実際の同期は繋がっていない）。

## 未決事項

本書は「どこに置くか」だけを決めている。以下は着手前に別途設計する。

- 認証方式とアカウントの扱い（`Account` モデルと
  `flequit-core` の `load_current_account` facade は既にある）
- 同期の粒度と転送形式（Automerge の変更をそのまま送るのか、REST API を切るのか）
- 同期の起動契機（起動時 / 変更時 / 定期 / 手動）と、失敗時の再試行
- サーバ側の実装言語とホスティング。別リポジトリになる可能性がある
- 複数の同期先が同時にある場合の順序と、片方だけ失敗したときの扱い

## 関連

- [`automerge-structure.md`](./automerge-structure.md): Automerge のドキュメント構造
- [`automerge-repo-dataflow.md`](./automerge-repo-dataflow.md): 保存と同期のデータフロー
- [`data-security.md`](./data-security.md): 暗号化とアカウント情報の扱い
- [`../architecture.md`](../architecture.md): クレート構成と依存方向
