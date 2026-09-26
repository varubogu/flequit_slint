# Flequit (Slint) 作業計画

- 最終更新: 2026-09-20
- 正本: 本ファイル。完了した計画はクリアし、判断の詳細は各仕様書と `docs/ja/` に残す

## 1. 検索ボックスを中心にしたタスク一覧

- 設計（正本）: `docs/ja/develop/design/ui/page/main/search.md`
- 検討の経緯: [検索ボックスへの連携方法.md](検索ボックスへの連携方法.md)（2026-09-19 確定）

### 1.1 手順

- [x] 設計書へ反映（`page/main/search.md` を新設し、`page/main/main.md` から参照）
- [x] 検索ロジック（`crates/flequit-ui/src/viewmodels/search/`）
  - [x] 語彙表（期限・状態・種類名の多言語表記）
  - [x] 字句解析（NFKC・全角記号・引用）と構文解析（AND / OR / NOT / 括弧、回復つき）
  - [x] 名前の解決（期限 / 状態 / プロジェクト / リスト / ユーザー、`*` の部分一致）
  - [x] クエリ文書（表示文字列と確定済み参照の対応、編集への追従、内部クエリの保存形式）
  - [x] 評価（タスクとサブタスクを 1 件ずつ評価、継承の規則）
  - [x] 編集操作（置き換え / AND / OR / NOT 追加 / 取り除き）とハイライト
  - [x] 入力候補と曖昧さ解消の候補
- [x] ViewModel（`app.rs`、`app/query.rs`）
  - [x] サイドバーの選択状態による絞り込みを廃止し、クエリだけで一覧を作る
  - [x] サイドバー操作（修飾キー・コンテキストメニュー）とハイライト
  - [x] 曖昧さ解消プルダウン、警告表示
  - [x] サブタスクの一致表示（薄く表示・自動展開・一致の印）
  - [x] 状態ボタン（未完了 / 完了）と件数
  - [x] タスクの追加先ピッカー
  - [x] ユーザーの読み込み、言語切替・改名への追従
  - [x] クエリと追加先の保存・復元（`UserSettings` → `flequit-settings` の `searchQuery` / `recentTaskDestinations`）
- [x] UI（`.slint`）と翻訳（`.pot` / en / ja）
- [x] テスト（検索ロジック・ViewModel の単体テスト、`tests/interaction.rs`、設定の往復）
- [x] 検証（fmt / clippy -D warnings / workspace test / 依存検証 / スキル同期）
- [x] 実機での動作確認（2026-09-19 ユーザー確認。入力候補でフォーカスが外れる問題を修正済み）

### 1.2 残作業

- [ ] サイドバー項目をキーボードでフォーカスできるようにし、主修飾キー + `Enter`（AND）、
      副修飾キー + `Enter`（OR）、`Menu` キー（コンテキストメニュー）を割り当てる
- [ ] iOS で `ContextMenuArea` が長押しに反応するか確認する（Phase 2 のモバイル対応で）
- [x] 旧実装の `crates/flequit-ui/src/viewmodels/search/candidate.rs` を削除する

### 1.3 実装時の判断

- 期限キーワードは「名前付き（今日・今年度など）/ 日数 / 分数」を区別して持つ（`DueSpec`）。
  日数に丸めると `@今年度` が `@365日` として書き戻されるため
- 「入力中」の語は直前の編集位置で判定する。`LineEdit` はキャレット位置を公開しない
- `-` の直後が確定済みトークンのとき、字句解析はセグメントごとに行うため `-` が単独扱いになる。
  構文解析側で「テキストの末尾の `-` + 次が確定トークン」を NOT として扱う
- `*` を含む語が何も指さないときは、他の未解決の `@` と同じく無視して警告する（一覧を空にしない）
- サブタスクの `completed` フラグ（旧フィールド）が立っていれば、状態が追いついていなくても完了として扱う
- タスク自身が当たったときはサブタスクに一致の印を付けない（所属・自由語を継承するため、印が意味を持たなくなる）
- サイドバーの件数は「そのボタンだけをクエリにしたときに出る行の数」で、サブタスクだけが当たる行も数える
- 修飾キーは Slint の `KeyboardModifiers` をそのまま使う。Apple 系で Command と Ctrl を入れ替えて渡すため、
  `flequit-platform` での OS 判定は不要だった
- クエリの保存は設定の保存経路を使うが、設定画面の再描画は行わない（`SearchMemory`）。
  1 キーごとに期限ボタンのモデルを作り直さないため
- 手で選んだ追加先は「選んだ時点の既定値」と組にして持ち、クエリが変わって既定値が変わったら破棄する
- 追加直後のタスクは、クエリに合わなくても次にクエリを変えるまで一覧に残す（見えなくなるのを避ける）
- 入力候補は `PopupWindow` をやめ、検索ボックスの直下にインライン表示する（2026-09-19 実機確認で修正）。
  ポップアップが開くとフォーカスを奪い、`@` の直後に入力欄からフォーカスが外れていたため

### 1.4 開発環境メモ

- この Mac では Xcode のライセンスが未同意のため、`cc` によるリンクが失敗する。
  `DEVELOPER_DIR=/Library/Developer/CommandLineTools` を付けると Command Line Tools でビルドできる。
  恒久的には `sudo xcodebuild -license` で同意する

## 2. Automerge 同期キュー（書き込みの応答速度改善）

- 設計（正本）: `docs/ja/develop/design/data/automerge-sync-queue.md`
- 目的: Automerge の保存（ドキュメント全体の読み書き）を書き込みの応答から外す

### 2.1 手順

- [x] 設計書を新設し、関連文書（トランザクション管理・データフロー・ルール・要件）を更新
- [x] キューテーブル `automerge_sync_queue`（マイグレーション・エンティティ・SQLite リポジトリ）
- [x] SQLite リポジトリの書き込みに `*_with_txn` を追加（統合リポジトリが同じトランザクションで使う）
- [x] 統合リポジトリの書き込みを「キュー登録 + SQLite」の 1 トランザクションへ切り替え
- [x] 削除（`TransactionalDeletionPort`）をスナップショット復元方式からキュー方式へ
- [x] 復元を `TransactionalRestorePort` へ移し、Automerge を読む前に未反映分を反映（読み取りバリア）
- [x] タグブックマークを `TagBookmarkRepositoryPort` 1 つに統合（Automerge はキュー経由）
- [x] ワーカー（id 順・ドキュメント単位の順序保証・指数バックオフ・10 回で failed・30 日で processed を削除）
- [x] `flequit-app` で起動、終了時に最大 3 秒まで反映
- [x] テスト（`crates/flequit-infrastructure/tests/automerge_sync_queue.rs`）
- [ ] 実機での動作確認（応答速度の体感、終了・再起動をまたいだ反映）

### 2.2 残作業

- [ ] `failed` の行を調べて再投入する手段（診断画面またはログ出力）
- [ ] アプリ終了時に Automerge-Repo を停止（`RepoHandle::stop`）し、ファイル保存の完了を待つ
- [ ] モバイルでバックグラウンドへ移るときにキューを反映する（ライフサイクル通知から起こす）
- [ ] 使われなくなったスナップショット系の port（`AutomergeProjectRepositoryPort` の
      `create_snapshot` / `restore_from_snapshot`、`AutomergeRepositoriesPort::projects_repo`）の整理
- [ ] 将来の端末間同期では、送信前に対象ドキュメントのキューを `flush_document` で反映する

### 2.3 実装時の判断

- 読み取りは SQLite だけなので、Automerge が遅れても画面には影響しない。Automerge を読むのは
  ゴミ箱からの復元だけで、そこには読み取りバリアを置いた
- キューの INSERT をトランザクションの最初の文にする。SQLite の読み取り→書き込みの昇格が
  ワーカーの更新と重なると、待たずに `SQLITE_BUSY` になるため
- 「反映成功」は Automerge ドキュメントへの適用成功とした。Automerge-Repo 0.3 は
  ファイル保存の完了を通知しない（失敗もログのみ）ため、それ以上は確認できない
- 入力が原因のエラー（ペイロードを読めない等）は再試行せずすぐ `failed` にする。
  再試行待ちで同じドキュメントを 10 分以上止めないため
- `SubtaskRecurrenceRepositoryTrait` の `save` / `delete_by_*` は Automerge 側が未対応
  （以前は Automerge 側のエラーで操作全体が失敗していた）。SQLite だけに書き、キューには入れない
- （2026-09-22）反映が 1 件約 45 秒かかり、終了時にフリーズしていた。原因は開発ビルドで
  Automerge が最適化なし・debug assertions ありだったことと、集合をリストで丸ごと書き直していたこと。
  開発ビルドでも `automerge` / `hexane` を最適化し、集合を「ID → エンティティ」の Map にして
  差分だけを書く形に改めた（`docs/ja/develop/design/data/automerge-structure.md`）。
  旧リスト形式は読み取り時にそのまま受け付け、最初の書き込みで Map に変換する。
  終了時は Tokio ランタイムの停止を 1 秒で打ち切る

