# Flequit (Slint) 移植計画

SvelteKit + Tauri 版 [`varubogu/flequit`](https://github.com/varubogu/flequit) から
Rust + Slint への移植における残作業の記録。

- 最終更新: 2026-09-13
- 正本: 本ファイル。設計の詳細は `docs/ja/` を参照する
- 完了した項目はチェックを入れ、判断が変わった項目は理由を残す

---

## 1. 現状サマリ

| 領域 | 状態 |
| --- | --- |
| ドキュメント (`docs/ja/`) | 移植完了（48 ファイル）。`docs/en/` は未作成 |
| エージェント設定 | 完了（Claude / Codex / Cursor、同期スクリプト + CI 検証つき） |
| Rust クレート（ドメイン・永続化） | 移植元からそのまま流用。Tauri 依存なし |
| `flequit-platform` | デスクトップ実装済み。Android / iOS はスタブ |
| Slint UI シェル | サイドバー / タスク一覧 / タスク詳細 / サブタスク詳細、レスポンシブ 3 段階、SVG アイコン |
| タスク操作 | 追加・完了・タイトル/ノート/期限/開始日時/優先度/ステータス編集・サブタスクの作成/編集/削除 |
| プロジェクト / リスト | 作成・リネーム・削除、プロジェクトのアーカイブと色設定 |
| 並び順 | ソート 4 種 + ドラッグ&ドロップによる並び替え / リスト間移動 |
| 検索 | キーワード構文（期限 / 属性 / タグ / 自由語）、入力候補、期限フィルタの件数表示 |
| 設定 | 言語 / 週開始 / 期限フィルタ / 繰り返しプリセット / 日時 / 外観（フォント列挙つき） |
| タグ | 作成・改名・色変更・削除、タスクへの付与 / 解除、サイドバーへのブックマーク |
| 繰り返し | 単位 / 間隔 / 曜日 / 月内の日 / 終了条件の編集と、次回以降の日時プレビュー |
| アクセシビリティ | `accessible-*` に加え、共通ボタンのフォーカス表示とモーダルのフォーカストラップ |
| ロギング | 標準エラー出力 + `log_dir()` 配下の日次ローテーションファイル（7 世代）。Android は logcat、Web は console |
| 検証 | `fmt` / `clippy -D warnings` / 371 tests（+ 1 ignored）/ 依存不変条件 / スキル同期 / `cargo audit` すべて green |

> 2026-09-07 の棚卸しで、チェック済みだが設計書の記載を満たしていない項目を
> 4.1 / 4.5 / 4.8 / 4.10 / 4.11 へ追加し、同日中にすべて実装した。

### 検証コマンド

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
./scripts/check-crate-deps.sh
./scripts/sync-agent-skills.sh --check
./scripts/test-prepare.sh && cargo test -j 4 --workspace
cargo audit
```

---

## 2. 確定済みの設計判断

再検討の前にここを読むこと。覆す場合は理由を追記する。

| 判断 | 内容 | 経緯 |
| --- | --- | --- |
| Slint 単体 | Tauri は併用しない | Tauri 2 は WebView 前提で UI 差し替え不可。`tauri-plugin-*` も `tauri::Runtime` 依存 |
| クレートは流用 | ドメイン・永続化はコピーして再利用 | 移植元に Tauri 依存が無く、そのまま動いた |
| ja 先行 | `docs/ja/` のみ整備 | `docs/en/` は実装が固まってから別タスク |
| PC 先行 | モバイルは Phase 2 | 設計・抽象化は最初から両対応で記述済み |
| 幅で分岐 | レスポンシブは OS 判定を使わない | デスクトップ上でレイアウト検証が完結する |
| Web 版は UI のみ | wasm32 ビルドはシェルの描画までとし、処理を持たせない | 保存はいずれバックエンドサーバが担うため、ブラウザ内に永続化層を作る必要がない |
| 保存先はユーザーが選ぶ | ローカル / クラウドストレージ / バックエンドサーバ | デスクトップもモバイルも「ローカル専用」を前提にしない。詳細は 8. |

---

## 3. P0 — ブロッカー

### 3.1 削除系 facade からストレージ固有型を除去する

- **状態**: 完了（2026-09-06）
- **対応**: `TransactionalDeletionPort` を追加し、SQLite のトランザクション型、
  削除順序、Automerge のスナップショット復元を `flequit-infrastructure` 内へ移動した
- **結果**: task / task list / project / tag の削除 facade は
  `Result<T, ServiceError>` を返し、ストレージ固有の境界を要求しない。
  task の削除ボタンも ViewModel へ接続済み
- **残りの UI**: subtask / project / tag の削除導線はすべて接続済み
  （それぞれ 4.1 / 4.2 / 4.4 を参照）
- 参照: `docs/ja/develop/design/ui/core-bridge.md` の「削除系 facade のトランザクション境界」

### 3.2 facade の戻り値を `Result<T, ServiceError>` へ移行する

- **状態**: 完了（2026-09-06）
- **対応**: 全 facade（13 ファイル）を `Result<T, ServiceError>` へ統一し、
  `handle_service_error` と文字列への丸めを廃止した
- **結果**: ViewModel の `UiError` 変換で「見つからない」「検証エラー」
  「ストレージ障害」を個別の i18n コードへ分類して表示できる
- 参照: `docs/ja/develop/design/backend/rust-guidelines.md` の「エラーハンドリング」

---

## 4. P1 — ver1.0 相当の機能

ロードマップ ver1.0（`docs/ja/roadmap.md`）に必要な UI 機能。

### 4.1 タスク管理

- [x] サブタスク作成
- [x] タスク削除
- [x] 期限の設定・編集（日付・時刻の選択、変更、クリア）
- [x] 優先度の設定（詳細ペインの4段階ボタン、楽観的更新・失敗時ロールバック）
- [x] ステータス変更（詳細ペインの5状態ボタン、完了状態との同期、楽観的更新・失敗時ロールバック）
- [x] タスクのソート（一覧上部のバーで 手動 / 期限 / 優先度 / 名前 を切替）
- [x] ドラッグ&ドロップによる並び替えとリスト間移動
      （行左端のハンドル。サイドバーのリストへドロップで同一プロジェクト内を移動）
- [x] サブタスクの改名・削除（3.1 で「残りの UI」としていた subtask の削除導線）
      詳細ペインの行から完了トグル・削除・オープン、サブタスク詳細から改名ができる
- [x] サブタスク詳細の表示（`SubTaskDetailView`。`AppState.selected-subtask` を
      ViewModel が解決し、`go-to-parent-task` が親タスクへ戻す）

#### 実装時の判断（2026-09-06）

- 並び順は安定ソートで、どのモードでも保存順（`order_index`）がタイブレーク。
  「手動」は保存順そのものなので比較は常に `Equal` を返す
- 期限なしのタスクは期限ソートで末尾。優先度は数値の降順
- 並び替えは「手動」のときだけ有効。派生した並びから `order_index` を
  書き換えると、画面上で動いていない行が動くことになるため
- ドロップ位置は行の高さを一定と仮定して求める（自動スクロールと同じ近似）
- 保存位置は「ドロップ後に前後へ来た可視行」から決める。検索で一部が
  隠れていても、隠れた行を巻き込まずに済む
- リスト間移動は同一プロジェクト内のみ。タスクはプロジェクト単位で
  保存されており、プロジェクトをまたぐ移動は別の操作になる
- 並び順の設定はセッション内のみ保持（永続化は user-preferences 側の課題）
- ドラッグはポインタ操作なので、ハンドルに increment / decrement の
  アクセシビリティアクションを持たせ、1 段ずつの移動を等価に用意した

### 4.2 プロジェクト / タスクリスト管理

- [x] プロジェクトの作成・リネーム・削除・アーカイブ
- [x] タスクリストの作成・リネーム・削除
- [x] プロジェクトの色設定（固定パレットから選択し、サイドバー行に描画）

#### 実装時の判断（2026-09-06）

- プロジェクトとタスクリストは共通のモーダルエディターを使う。削除は配下の
  データも対象になるため、エディター内で確認を 1 段挟む
- ツリー構造を変える保存操作は、成功後にストレージから全体を再読込する。
  サイドバー、選択、タスク一覧の複製状態を個別に同期しない
- アーカイブ済みプロジェクトは通常非表示とし、サイドバーフッターから表示を
  切り替える。非表示になる項目が選択中なら、有効な先頭項目へ選択を移す
- 色はライト / ダーク双方で視認できる 7 色の固定パレットとする。
  保存値は hex 文字列、描画値は Adapter で変換した `brush` を使う
- 新規項目の `order_index` は件数ではなく既存最大値 + 1 とし、削除後や
  欠番がある場合も既存項目と衝突させない

### 4.3 検索

- [x] キーワード構文の解析（`viewmodels/search/`。トークンは空白区切りの AND）
  - 期限: `@today` / `@今日` / `@overdue` / `@3days` ほか、`@<数値><単位>` も対応
  - 属性: `@proj:` / `@project:` / `@list:` / `@task:` / `@note:` /
    `@subtask:` / `@subtasknote:`
  - タグ: `#tag`（前方一致。タグ名は読み込み時に解決してキャッシュする）
  - 自由語: タスク名 / ノート / サブタスク名 / サブタスクノートの部分一致
- [x] 入力候補の表示（`@` / `#` 入力時）
- [x] 期限フィルタボタンの件数表示
      （選択中のプロジェクト / リストを対象に、検索文字列とは独立に数える）
- 参照: `docs/ja/develop/design/ui/page/main/main.md`

#### 実装時の判断（2026-09-06、2026-09-07 追記）

- 期限キーワードの日数は「今日を 1 日目として N 日」に統一した。
  `@3days` = 今日・明日・明後日、`@2日` = `@明日`。仕様書の例と一致する
- `@今期` / `@今年度` は期間開始日・年度開始日の設定が未実装のため、
  今日から 90 日 / 365 日で近似している。設定を持てるようになったら見直す
- 未知の `@キーワード` は無視する。仕様上はアカウント名だが未解決であり、
  `@to` のような入力途中で一覧が空になるのを避けるため
- 期限フィルタは期限なしのタスクに一切マッチしない
- 入力候補は末尾のトークンだけを対象にし、選択時もそれ以前の AND 条件を保持する。
  `@` は期限・プロジェクト・リスト・現在のアカウントを部分一致、`#` はタグを
  検索本体と同じ前方一致で絞り込む

### 4.4 タグ

- [x] タグの作成・編集・削除 UI
- [x] タスクへのタグ付け / 解除
- [x] タスク行へのタグ表示（`TagChip`。タグ名は `load_tags` で解決）
- [x] タグブックマーク（サイドバー固定表示）

#### 実装時の判断（2026-09-06）

- タグはプロジェクト単位で管理し、選択中プロジェクトのタグをモーダルで
  作成・改名・色変更・削除する。削除時は全タスクから外れる旨を確認する
- タスク詳細にはプロジェクト内の全タグを表示し、既存タグの付与 / 解除と、
  名前入力によるタグ作成 + 付与を同じ領域から行えるようにした
- タグ名は検索構文の `#tag` と一致させるため空白を許可しない
- ブックマークはユーザー設定として保存し、全プロジェクト分をサイドバーへ固定表示する。
  選択時はタグ所属プロジェクトへ移動して `#タグ名` 検索を適用する
- タグや関連付けの変更後は canonical なプロジェクトツリー、タグ、ブックマークを
  再読込し、一覧・詳細・検索キャッシュ間の不整合を避ける

### 4.5 日時

- [x] 日付・時刻ピッカーコンポーネント
- [x] 日時フォーマット設定画面
      （設計は `docs/ja/develop/design/ui/page/settings/datetime-format.md` に完備）
- [x] タイムゾーン設定の反映
      （`system` / `UTC` / IANA タイムゾーンを表示・入力・期限検索へ適用）
- [x] 開始日時・終了日時の編集（「期間」トグルで `is_range_date` を切り替え、
      ON のとき `plan_start_date` のピッカーを出す）

#### 実装時の判断（2026-09-06）

- 日時は独立した設定カテゴリとし、タイムゾーン、現在書式、試行用書式、
  プリセット、カスタム書式をまとめた
- プリセットは日本（24 時間）、米国（24 時間）、ISO 8601 を組み込み、
  アプリ固定データとして設定ファイルへは保存しない
- カスタム書式は UUID を採番して即時保存する。追加・上書き・削除により
  現在書式が変わる場合はタスク一覧と詳細ペインも即時再描画する
- 不正な書式も設定値として保存する。プレビューではエラーを表示し、
  タスクの期限ラベルは組み込み書式へフォールバックして操作不能を避ける
- 不明な IANA タイムゾーンは `system` へフォールバックする

### 4.6 繰り返し

- [x] 繰り返しルールの設定 UI（日 / 週 / 月 / 年）
- [x] 繰り返しプレビュー
- [x] 完了時の次タスク生成と、キャンセル時の未変更の次タスク削除
- facade は実装済み（`recurrence_facades`: 18 関数）

#### 完了時の次タスク生成（2026-09-15）

仕様は `docs/ja/develop/design/data/entity/projects.md` の Task 補足「繰り返しタスクの次タスク」に置いた。

- タスクには `previous_task_id` だけを追加し、次タスク ID は持たない（Git の親コミットと同じ考え方）
- 生成と削除の判断は `flequit-core` の `recurring_task_service::sync_successor` に置き、
  ViewModel はステータスの保存に成功した後にこれを呼ぶ。
  次回日時の計算は表示タイムゾーンのカレンダーに依存するため、ViewModel の
  `next_due_after` を関数として渡す
- 後処理は保存済みのステータスを読み直して判断し、プロセス内でロックして直列に実行する。
  完了 → 未完了 → 完了と素早く切り替えても二重に生成しない
- 未完了へ戻す操作（キャンセル以外）は次タスクに触れない。仕様がキャンセルだけを定めているため
- 複数端末で同時に完了した場合の二重生成は、同期側の課題として残す

#### Automerge 書き込みの高速化（2026-09-15）

完了から次タスク表示まで数秒かかっていた。アプリログの時刻で切り分けると、表示（再読込）は
約 15 ms で、時間の大半は Automerge への書き込みだった（`set_task` 1 回 2.6〜4.3 秒、タスク 5 件）。

- 原因: `Document::put_json_value` が保存のたびにエンティティ配列を丸ごと新しい
  オブジェクトとして作り直していた。変更履歴が保存回数に比例して膨らみ、1 回の書き込みが
  次第に遅くなる（空のドキュメントでも 12 回目で 810 ms）
- 対策: 既存の値との差分だけを書く。同じ値は書かず、マップとリストは既存オブジェクトを
  その場で更新する（リストは位置で突き合わせる）。実データのコピーでステータス更新は
  14〜31 ms、タスク追加は 217 ms になった
- 開発ビルドでは `automerge` / `automerge_repo` だけ最適化する（`Cargo.toml` の
  `profile.dev.package`）。未最適化の automerge はリスト操作ごとに検証用の全走査を行い、
  差分書き込み前の計測で約 3.5 倍遅かった
- Automerge 書き込み中は SQLite のトランザクションが開いたままになるため、書き込みが遅いと
  並行する保存が `database is locked` で失敗していた（次タスクが生成されない原因の一つ）。
  書き込み時間の短縮で窓は小さくなったが、トランザクション中に Automerge を待つ構造自体は残る

#### 実装時の判断（2026-09-06）

- 単位は計画の 日 / 週 / 月 / 年 に加えて 分 / 時 / 四半期 / 半年 も選択可能にした。
  ドメインの `RecurrenceUnit` が持つ全単位を UI 側の enum に写しておかないと、
  他クライアントが作成したルールを開いて保存した時点で単位が失われるため
- 繰り返し計算（次回日時の展開）は `flequit-ui` の ViewModel に置いた。
  `docs/ja/develop/design/ui/slint-patterns.md` が繰り返し計算を ViewModel 側の
  責務として明示しており、`.slint` には結果の文字列だけを渡す
- 展開は表示タイムゾーンのカレンダー上で行う。UTC のまま加算すると
  「毎日 9:00」が夏時間の切り替えで 1 時間ずれる
- 起点は対象タスクの期限。期限未設定のタスクは「今」を起点にプレビューする
- `max_occurrences` は起点を 1 回目として数える。したがってプレビューに出る
  未来の日時は最大 `max_occurrences - 1` 件になる
- 該当日が存在しない周期（31 日指定の 2 月、第 5 水曜が無い月）はクランプせず
  スキップする。ユーザーが指定していない日に勝手にずらさないため
- 営業日補正（`RecurrenceAdjustment`）と日付条件（`DateCondition`）は編集 UI を
  持たないが、保存時にそのまま引き継ぐ。祝日カレンダーが未実装のため
  祝日系の `AdjustmentTarget` は補正せず元の日付のままプレビューする
- 終了条件は「終了しない / 指定日まで / 指定回数まで」の排他選択とする。
  両方を持つ既存ルールは指定日として開き、保存し直すまで回数は失われない
- ルールの保存は `create_recurrence_rule`（ルール ID 指定の upsert）を
  新規・更新の双方で使う。`update_recurrence_rule` の patch も内部では
  find → apply → save であり、同じ経路を 2 通り書き分ける意味がないため

### 4.7 リマインダー

- [x] タスクへのリマインダー設定 UI（任意数）
- [x] `flequit-platform::notification` への接続
- [x] 通知許可のリクエスト導線（設定画面）

#### 実装時の判断（2026-09-06）

- リマインダーは期限からの相対値ではなく UTC の絶対日時をタスクへ任意数保持する。
  期限変更でユーザーが指定した通知日時まで暗黙に動かさないため
- SQLite は日時配列を JSON 列へ保存し、既存データはマイグレーションで空配列にする。
  Automerge は `serde(default)` により旧ドキュメントを同じ形で読み込む
- 通知許可は初回リマインダー追加時に要求し、拒否後の再要求導線を基本設定へ置く。
  デスクトップでは許可済みとして扱う
- デスクトップの予約タイマーは `flequit-platform` が所有し、起動時に未来の予約を
  復元する。Android / iOS の OS スケジューラ接続は Phase 2 の項目で実装する
- リマインダー削除とタスク削除では安定 ID（タスク ID + UTC 日時）から予約を取り消す

#### 入力 UI の見直し（2026-09-09）

- リマインダー追加は、設定済みの着手日時または完了日時から逆算する選択肢を
  先に提示する。候補は「30分前・1時間前・1日前」を初期値とし、設定の
  「リマインダーのカスタム項目」で分・時間・日単位の候補を追加・削除できる
- カレンダーから絶対日時を指定する入力は「オプション」と明示して残す
- リマインダーのカレンダーだけ、設定タイムゾーンの今日より前を選択不可とし、
  過去日時・現在時刻の入力は閉じる前に拒否する。共通カレンダーには呼び出し元が
  日付の選択可否と確定値の検証を差し込めるようにし、他のカレンダーは制限しない
- 永続化形式は引き続き UTC の絶対日時とする。相対条件そのものを保持して
  タスク日時の変更へ追従させる方式への移行は、保存形式の仕様確定後に行う

### 4.8 設定画面

- [x] 設定モーダルの実装
  - 基本設定（曜日開始日、期限ボタンの表示有無、カスタム期限フィルタ追加）
  - 外観設定（テーマ、フォント、フォントサイズ、配色）
  - アカウント設定（現在のローカルアカウント情報を表示）
- [x] `Capabilities` による項目の出し分け
- [x] `Compact` での 2 階層遷移
- [x] 基本設定「繰り返し設定のカスタム項目追加」
      （単位 × 間隔を登録し、繰り返しエディタがプリセットとして提供する）
- [x] フォント選択をインストール済みフォントの列挙にする
      （`Capability::FontEnumeration` + `fontdb`。絞り込みコンボボックス）
- 参照: `docs/ja/develop/design/ui/page/settings/settings.md`

#### 実装時の判断（2026-09-06）

- 設定値は `flequit-ui` 側の `SettingsStore` ポートを `flequit-app` が実装し、
  `flequit-settings` への直接依存を UI クレートへ持ち込まない
- 変更は画面へ即時反映し、単一キューで YAML へ保存する。保存失敗時は最後に
  保存できた値へロールバックし、設定用のエラーコードを表示する
- デスクトップでは全カテゴリを連続表示し、カテゴリ選択で該当位置へ移動する。
  Compact はカテゴリ一覧と詳細の二階層に分ける
- アカウントは現行のローカルアカウント情報を表示する。クラウド認証とプロフィール編集は
  アカウント基盤の対象であり、設定モーダルでは将来対応の案内に留める
- テーマ・配色の描画反映と OS ダークモード追従は 4.9 で追跡する。
  フォントは 2026-09-07 に一覧化と描画反映まで 4.8 の対象へ含めた
- カスタム期限フィルタは値と単位（分 / 時間 / 日）の組で保存する。分・時間の
  ホライズンは現在時刻からの相対で判定し、日単位のようにその日の終わりへ
  丸めない。「10分以内」が今日いっぱいを意味しては用をなさないため
- 旧形式（日数だけの配列）の設定ファイルも読めるよう、`CustomDueFilter` は
  整数を「日」として解釈する `Deserialize` を持ち、YAML キーには
  `custom_due_days` の別名を残す
- 設定ファイルは旧 Tauri 版（`varubogu/flequit`）と同じパス
  （`ProjectDirs` の config_dir 配下 `settings.yml`）を共有するため、
  YAML キーは旧実装と同じ **camelCase** で読み書きし、snake_case も
  `#[serde(alias)]` で受け付ける。`DateTimeFormatGroup` も旧実装に合わせて
  snake_case（`default` / `custom_format`）で保存する。
  旧アプリを使っていた環境で `missing field font_size` により起動できなく
  なるのを防ぐため（2026-09-07）
- タイムゾーンは絞り込みコンボボックスにした。IANA の候補は数百件あり、
  絞り込みは Rust 側（`viewmodels/settings/timezone.rs`）で行う。`.slint` の
  バインディングで毎キーストローク全件を走査させないため
- タイムゾーンは一覧から選ぶか Enter で確定した時だけ保存する。入力途中の
  「Asia/T」のような値が保存され `system` へフォールバックするのを避けるため

### 4.9 テーマ

- [x] テーマ切替の実装（`Theme.mode` から `Theme.dark` を導出）
- [x] OS のダークモード追従（`ThemeMode.system`、起動時に一度取り込む）
- [x] 設定の永続化（`flequit-settings`）
- [x] 起動後の OS テーマ変更への追従
- [x] フォント色・背景色設定の描画反映

#### 実装時の判断（2026-09-06）

- 色トークンは `Theme.dark` だけを見る。`Theme.dark` は `Theme.mode` と
  `Theme.system-dark` から導出する `out` プロパティで、Rust は `mode` のみ書き込む
- OS の配色は `AppWindow.init` で `Palette.color-scheme` から読み取り
  `Theme.system-dark` へ退避する。以降は `Theme.dark` の変化に合わせて
  `Palette.color-scheme` を書き戻し、std-widgets も同じ配色で描画させる
- 起動後の変更は `flequit-platform` の `SystemThemeWatcher` で検出する。
  デスクトップ実装は `dark-light` を使い、ViewModel の専用スレッドから
  `upgrade_in_event_loop()` 経由で `Theme.system-dark` を更新する
- フォント色・背景色の選択値は `Theme` の色トークンへ反映する。旧既定値の
  `#000000` / `#FFFFFF` は固定色ではなく `default` と同様に扱い、テーマ追従を維持する

### 4.10 UI シェルの未完部分

- [x] 自動スクロール（タスク選択時に `AppState.scroll-to-index` を発行・消費）
- [x] サイドバー折りたたみをデスクトップで機能させる
- [x] アカウントボタン（サイドバーフッター、メニュー表示）
- [x] タスク一覧の空状態からの導線改善
- [x] サイドメニューのアイコンを SVG にする
      （`ui/assets/icons/` の 19 個を `components/icon.slint` の `Icon` が
      テーマ色で描画する。シェル全体の文字アイコンも同時に置き換えた）

#### 実装時の判断（2026-09-07）

- ver1.0 はローカルアカウントのみのため、アカウントメニューは設定、アカウント設定、
  ヘルプを提供する。ログイン、ユーザー作成、ログアウトはクラウド認証と同時に追加する
- 空状態は検索 0 件、プロジェクトなし、リスト未選択、選択リストにタスクなしを区別し、
  検索クリア、プロジェクト作成、リスト作成、クイック追加へのフォーカスを直接提供する

#### 実装時の判断（2026-09-07）

- サブタスクは親と同じスキーマを持つが、詳細ペインで扱うのは名前・期限・ステータス・
  優先度・ノートに絞る。タグと繰り返しはタスク側の概念として残す
- サブタスクの編集は楽観的更新、削除だけは再読込にする。親の進捗カウントは
  派生値で、再読込しないと数え直せないため
- 期間は `is_range_date` の切替で表現する。OFF に戻すときは開始日時も消す。
  隠れた値が次の ON で復活すると、ユーザーが指定していない期間になる
- 開始日時が保存されているタスクは、フラグが無くても期間として開く。
  他クライアントがフラグ無しで書いた場合に開始日時が編集できなくなるため
- 実績日時（`do_start_date` / `do_end_date`）は ver1.1 の「着手・完了時間」の
  対象とし、ver1.0 では UI を持たない
- フォント一覧は `flequit-platform` の `available_fonts()` 越しに取得し、
  専用スレッドで走らせる。フォントディレクトリの走査は UI スレッドには重い
- フォントの選択は `AppWindow.default-font-family` に流す。全 `Text` へ
  `font-family` を書くより変更点が 1 か所で済み、std-widgets にも効く
- アイコンは 1 色の SVG を `colorize` で塗る。ライト / ダークで資産を
  分けずに済み、フォント色の設定にも追従する
- `@image-url` はコンパイル時に解決されるため、アイコン名は `IconName` に
  列挙する。文字列からパスを組み立てることはできない

### 4.11 言語切替

`docs/ja/develop/design/ui/i18n-system.md` は「設定画面での言語選択後は即時反映」と
定めている。切替は永続化を伴うため `Actions.update-language` に一本化し、
呼び出し元が無かった `I18n.locale-changed` は廃止した。

- [x] 設定画面（基本設定）へ言語選択を追加する
- [x] 選択値を `flequit-settings` の `language` へ永続化する
- [x] 起動時に保存値を適用し、未設定ならシステムロケール、未対応なら `en` へ

#### 実装時の判断（2026-09-07）

- 言語は `Actions.update-language` だけを入口にする。翻訳の切替と保存を
  1 か所にまとめないと、保存されない切替と保存される切替が併存する
- 設定ファイルの `language` は 2 文字コードしか受け付けない（`validation.rs`）。
  「システムに従う」は保存できないため、解決後のタグを書き戻す
- システムロケールは環境変数から読む。プロセスが既に持っている値であり、
  これだけのために `flequit-platform` へ OS 呼び出しを足す必要はない
- 地域・エンコーディングの接尾辞は捨てて言語サブタグで照合する
  （`ja_JP.UTF-8` → `ja`）。カタログは言語単位のため

---

## 5. P2 — 品質・基盤

### 5.1 アクセシビリティ

- [x] キーボード操作の実装（要件は全機能をキーボードで操作可能）
  - タスク間移動、ペイン間移動、ショートカット（Ctrl+N ほか）
  - vim モード（オプション）
- [x] フォーカス表示とフォーカストラップ
- [x] `accessible-role` / `accessible-action-default` の付与
- 参照: `docs/ja/develop/requirements/accessibility.md`

#### 実装時の判断（2026-09-08）

- `TouchArea` だけの要素は Tab で到達できないため、共通ボタン
  （`IconButton` / `DialogButton` / `RowButton` / `ChoiceButton` / `ColorPicker`）に
  `FocusScope` を持たせ、Space / Enter で発火するようにした。
  フォーカス表示は `Theme.focus-ring` を使う `components/focus-ring.slint`
- フォーカストラップは `components/focus-sentinel.slint` を
  ダイアログのカードの最初と最後に置き、端で互いへ折り返す方式。
  Slint には「次の要素へフォーカスを進める」API がないため、
  折り返し直後は反対側の番兵にフォーカスが載る（Tab をもう一度押すと
  ダイアログ内の端の要素へ入る）。フォーカスが外へ出ないことを優先した
- ダイアログを開いた時点で内部の要素へフォーカスを置く
  （名前入力、繰り返しのチェックボックス、設定の閉じるボタン、タグの閉じるボタン）。
  削除確認のように `if` で要素が入れ替わる箇所は、切り替え先にも `init` フォーカスを置く。
  フォーカスを持っていた要素が消えると行き先を失って外へ出るため
- 検証は `interaction.rs` の `a_modal_keeps_keyboard_focus_inside_itself`。
  Tab のたびに Down を送り、背後のタスク一覧が選択を動かさないことを見る
  （トラップを外すと落ちることを確認済み）

### 5.2 テスト

- [x] ViewModel の単体テスト拡充（現在は Adapter と展開状態のみ）
- [x] 操作の結合テストにケース追加（新規 UI ごとに必須）
- [ ] ブレークポイント境界のスクリーンショット比較の仕組み
- [x] **ヒットテストの自動検証手段**（2026-09-08）
- 参照: `docs/ja/develop/design/testing.md` の「操作の結合テスト」

#### 実装時の判断（2026-09-08）

- ViewModel 側は `SharedState` の選択解決（`ensure_default_selection`）、
  表示対象の絞り込み（`tasks_in_scope`）、リマインダー収集
  （`reminder_specs_from_trees`）と、再読込の合流（`reload_gate`）を追加した。
  いずれも Slint ウィンドウを作らずに動く
- 結合テストには、モーダルのフォーカストラップと `ListView` の仮想化の 2 ケースを追加。
  どちらもキーイベント（`WindowEvent::KeyPressed`）を使うため、
  ウィンドウを `WindowActiveChanged(true)` にしないとイベントが届かない
- 「操作の結合テストにケース追加」は新規 UI ごとに継続して必要。
  チェックは 2026-09-07 時点の UI をすべて覆っていることを示す
- スクリーンショット比較は未着手。フォント描画が OS で変わるため、
  基準画像をリポジトリに置く方式は環境差で落ちる。CI で走らせる前提の設計が要る。
  なお `i-slint-backend-testing` の `renderer-software` フィーチャ（公開）で
  `TestingBackendOptions::renderer_name` を指定すればヘッドレス描画と
  `Window::take_snapshot()` は動く。決定的なフォントを入れる
  `configure_test_fonts()` だけが `internal` 側にあり、そこが未解決

#### ヒットテストの判断（2026-09-08）

- `send_mouse_click`（任意座標）は `internal` のままだが、
  **`ElementHandle::mock_single_click` は公開 API** で、要素の中心へ実際の
  `PointerPressed` / `PointerReleased` を送る。上流を待つ必要はなかった
- アクセシビリティ経由の駆動は要素を直接指すためジオメトリを見ない。
  つぶれた要素も、他の `TouchArea` の下に埋まった要素も動いてしまう。
  そこを埋めるのがヒットテスト側のケース
- 追加したのは 4 件（素のクリック到達、ダイアログのスクリム、compact の
  サイドバーオーバーレイ、読み込みベール）。「飲む」3 件は対応する
  `TouchArea` を消すと落ちることを確認済み
- ダイアログのスクリムは最初 **設定ダイアログ** で書いたが、
  カードが画面をほぼ覆うため、スクリムを消しても落ちなかった
  （カード内の要素がクリックを受けていた）。中央の小さなカードである
  プロジェクト編集ダイアログに変えて、スクリムだけが覆う位置を突くようにした
- ウィンドウ幅は `Layout.window-width` を直接書かず `set_size()` で変える。
  実ジオメトリと分岐がずれた状態でヒットテストしても意味がないため
  （テスト用ウィンドウの既定は 800x600）

### 5.3 パス・設定の受け渡し

- [x] `UnifiedConfig` にディレクトリを渡せるようにする
- [x] `flequit-settings` の設定ディレクトリを `flequit-platform::paths` 起点にする
- [x] `crates/flequit-repository/src/utils/path_service.rs` の削除

#### 実装時の判断（2026-09-07）

- `UnifiedConfig::with_storage_paths` で SQLite ファイルと Automerge ディレクトリを明示し、
  統合リポジトリ内の全 SQLite Adapter が同じ `DatabaseManager` を共有する。
  プロセス全体へ影響する `FLEQUIT_DB_PATH` / `FLEQUIT_AUTOMERGE_PATH` は廃止した
- `flequit-settings` は OS のパスを解決せず、呼び出し元から設定ディレクトリを受け取る。
  `flequit-app` が `platform.paths().config_dir()` を渡すことで依存方向を維持する
- テスト出力先は `CARGO_MANIFEST_DIR` からリポジトリ内 `.tmp/tests/cargo` を導出する。
  実行ディレクトリによってワークスペース外へ逸脱しないようにした

### 5.4 ロギング

- [x] ファイル出力（`platform.paths().log_dir()` へローテーション）
- [x] Android は logcat へ切り替え（P3 のモバイル実装とあわせて実施）

#### 実装時の判断（2026-09-08）

- `flequit-app::init_logging` が標準エラー出力とファイルの 2 層を組み立てる。
  ファイルは `tracing-appender` の日次ローテーション（`flequit.<日付>.log`、7 世代）
- 返る `WorkerGuard` は `run()` が握る。先に drop するとバッファが flush されない
- ログディレクトリを開けない場合はファイル出力だけ諦めて起動する。
  画面にログが出ないより、ディスクに残らない方が軽い
- モバイルのシンクは `cfg(target_os)` を必要とし、それは `flequit-platform` にしか
  置けない。`flequit-platform::SystemLogWriter` として用意し、`flequit-app` は
  `SystemLogWriter::current()` が `Some` を返したときだけ層を足す形で解決した
  （2026-09-08）。Android は `__android_log_write`、wasm32 はブラウザの console
- **iOS は OSLog にしない**。stderr が Xcode のコンソールにそのまま出るため、
  ブリッジを増やす価値がない。`current()` は `None` を返す

### 5.5 ドキュメント

- [ ] `docs/en/` の作成（`docs/ja/` が固まってから）
- [x] 未実装として注記したパスの解消（2026-09-08）
  - `data/partial-update-implementation.md`: `adapters/patch.rs` は作らないことにし、
    実際の実装（`viewmodels/app.rs` が操作単位で `PartialXxx` を組み立てる）を書いた
  - `data/user-preferences.md`: `viewmodels/user_preferences/` の新設をやめ、
    配線は `app.rs`、純粋ロジックは `viewmodels/<機能>/` という現状の規則に直した
  - `ui/viewmodel-architecture.md`: 命名・配置の表を実装に合わせ、
    エンティティごとの ViewModel 型を作らない理由を書いた
- [x] `i18n/flequit-ui.pot` の未使用エントリ（`msgid "Settings"`）を除去する

### 5.6 パフォーマンス

- [x] 変更のたびにプロジェクト全体を再読込している点の見直し
      （`reload_projects`。正確さを優先した暫定実装）
- [x] 大量タスクでの `ListView` 検証
- [ ] 起動時の初期クエリ件数上限

#### 実装時の判断（2026-09-08）

- 再読込は「全件を読み直す」ままにし、**回数** を削った
  （`viewmodels/reload_gate.rs`）。実行中の再読込があれば要求を畳み込み、
  連続した編集 N 回でも読み直しは 2 回で済む。最後の 1 回はすべての書き込みを見る
- 差分更新（変更されたプロジェクトだけ読み直す）は見送った。
  `flequit-core` にプロジェクト単位の facade がなく、
  検索・件数表示・並び替えが全ツリーをメモリに置く前提で書かれているため、
  正確さを崩さずに入れるには core 側の追加が要る
- `ListView` はタスク 500 件と 5000 件で生成される行数が変わらないことを
  `interaction.rs` で検証した（アクセシビリティツリーの list-item を数える）
- 起動時の件数上限は未着手。上と同じ理由で、上限を入れると検索と件数表示が
  「読み込んだ範囲だけ正しい」状態になる。ページングを core に入れてからの作業

---

## 6. P3 — Phase 2 モバイル

> **未検証**。以下の実装は Windows 機上で書かれており、Android SDK/NDK・Xcode・
> wasm ツールチェーンのいずれも無い環境のため、**モバイル / Web 向けには一度も
> コンパイルされていない**。C ドライブの空き容量不足で `rustup target add` すら
> 通らなかった（詳細は 10. 環境メモ）。デスクトップ側の回帰が無いことだけは
> `cargo test --workspace` で確認済み。初回 CI で修正が要ると見込むこと。

### 6.1 プラットフォーム実装

- [x] Android: サンドボックスルート取得（`Context.getFilesDir` を JNI 経由）
- [x] Android: 通知（チャネル + 実行時許可要求）
- [x] Android: SAF によるファイル選択（`FileHandle::Opaque`）
- [x] Android: `Intent.ACTION_VIEW`
- [x] iOS: サンドボックスルート取得
- [x] iOS: `UNUserNotificationCenter`
- [x] iOS: `UIDocumentPickerViewController`
- [x] iOS: `UIApplication.open`
- [x] ライフサイクル購読（`Suspend` / `Resume` / `LowMemory`）の実装と接続
- [ ] Android: `AlarmManager` によるプロセス外リマインダー
- [ ] Android: 通知許可ダイアログの結果を `onRequestPermissionsResult` から受け取る

#### 実装時の判断（2026-09-08）

- Android は `ndk-context` から `JavaVM` と `Activity` を取得する。
  `android-activity` を直接依存に入れると、Slint が使うバージョンと二重管理に
  なるため。初期化呼び出しも不要になった
- SAF とライフサイクルだけは Rust から取れない。結果が呼び出し元ではなく
  `Activity` に届くため。`FlequitActivity.java` が JNI で転送する。
  素の `NativeActivity` で動かした場合はメソッドが無いので
  `PlatformError::Unsupported` になる
- iOS は objc2 で UIKit を叩かず、**Swift ブリッジ（`mobile/ios/Sources/`）に
  C ABI 関数を置いて Rust から `extern "C"` で呼ぶ**。デリゲートと
  completion handler を Rust で組むと実行時まで誤りが出ないが、Swift なら
  Xcode が型検査する。加えてキーウィンドウはアプリターゲットからしか触れない
- iOS のコンテナパスだけは `HOME` から解決するのでブリッジ不要。ブリッジが
  リンクされていなくてもパス解決とログは動く
- Android の予約通知はプロセス内タイマーのまま。`AlarmManager` にするには
  `BroadcastReceiver` の追加が要る。iOS は `UNUserNotificationCenter` が OS 側で
  持つので、アプリが落ちても発火する
- `open_path` は両 OS とも `Unsupported`。Android は `FileProvider` 未宣言、
  iOS はコンテナ外から読めないため。capability ではなくエラーで返している
  唯一の箇所で、UI からは呼ばれない

### 6.2 ビルド基盤

- [x] `mobile/android/`（Gradle、マニフェスト、`FlequitActivity`）
- [x] `mobile/ios/`（XcodeGen `project.yml`、Swift ブリッジ、`main` シム）
- [x] `crates/flequit-app/src/entry_android.rs` / `entry_ios.rs`
- [x] CI のモバイルジョブを `continue-on-error` から外し、`flequit-app` まで検査
- [ ] アプリアイコン（現在は Android がフレームワークのプレースホルダ）
- [ ] SQLite のクロスコンパイル確認（`libsqlite3-sys` の `bundled`）— CI 初回実行待ち

#### 実装時の判断（2026-09-08）

- エントリポイントは `cfg(target_os)` ではなく **Cargo feature**（`android` /
  `ios`）で切り替える。`#[cfg(target_os = ...)]` は `flequit-platform` 専用という
  不変条件を崩さずに済む。ただし `slint::android` 自体が `target_os = "android"`
  で閉じているので、`--features android` はターゲット指定と併用でしか通らない
- `android_main` は `#[unsafe(no_mangle)]` で書く。`#[slint::android_main]` という
  マクロは存在しない（Slint 1.17 で確認）
- `AndroidApp` は `slint::android` の re-export を使う。バージョン追従が不要になる
- iOS は winit バックエンドが `UIApplicationMain` を自分で呼ぶため、Swift 側に
  `@main` もアプリデリゲートも置けない。`@_cdecl("main")` が唯一のエントリで、
  そこからライフサイクル監視を仕掛けて Rust に制御を渡す
- `crate-type` に `staticlib` を追加（iOS 用）。Android は既存の `cdylib`
- ログ出力先の選択は `flequit-platform::logging` に移した。Android は stderr が
  捨てられるので `__android_log_write` で logcat に出す。iOS の stderr は Xcode の
  コンソールに出るのでそのまま

### 6.3 実機検証

- [x] セーフエリア（`Window.safe-area-insets` を `Layout.safe-area-top/bottom` に接続）
- [x] セーフエリアの左右（横向きのノッチ）
- [ ] 慣性スクロール、ソフトキーボード
- [ ] 長押しメニュー
- [ ] バックグラウンド遷移時のデータ保全
- [ ] Android 実機 / エミュレータでの起動確認
- [ ] iOS シミュレータでの起動確認

#### 実装時の判断（2026-09-08）

- `Layout.safe-area-top/bottom` は宣言済みで消費側も揃っていたが、値を入れる側が
  無かった。Slint 1.17 の `Window.safe-area-insets` をそのまま流し込んで解決。
  デスクトップでは常にゼロなので OS 分岐は不要
- `changed` は初期値では発火しないため、`init` からも明示的に publish している
- 左右は `Layout.safe-area-left/right` に加えて、
  `content-safe-area-left/right`（compact のときだけ非ゼロ）を用意した。
  画面の横端を占める要素がブレークポイントで変わるため。非 compact では
  左端はサイドバー、右端は詳細ペインが占め、一覧ペインはどちらにも触れない
- インセットは各要素の padding に足す。外側の `HorizontalLayout` にまとめて
  padding を置くと、ノッチの下に背景色が届かず帯になる
- 設定ダイアログだけはカード自体を左右にずらす（`x` と `width`）。compact では
  画面いっぱいに開くが、中の 3 ペインすべてに padding を配るより、
  カードを縮めてノッチの下に背後のスクリムを見せる方が単純
- 上下と同じくデスクトップでは常にゼロなので、自動テストでは値が動かない。
  正しさは実機（6.3 の起動確認）でしか見えない

---

## 7. Web — UI のみ

`crates/flequit-web` + `web/`。実 `.slint` シェルをサンプルデータで描画する。
**Web 版はこれ以上の処理を持たせない方針**で、保存が要る段階になったら
ブラウザ内ではなくバックエンドサーバ（8.）に置く。したがって
IndexedDB / OPFS 上の Repository 実装は**作らない**。

- [x] `crates/flequit-web`（wasm32 向け UI ビルド、サンプルデータ）
- [x] `flequit-platform` の `WebPlatform`（capability 全部 false、console ログ）
- [x] `web/index.html` と手順（`web/README.md`）
- [x] CI に wasm32 ビルドジョブを追加
- [ ] バックエンド API クライアント（8. のサーバ設計が固まってから）
- [ ] Web 用のランタイム（`tokio` current-thread + `wasm-bindgen-futures`）

#### 実装時の判断（2026-09-08）

- `flequit-ui` は wasm32 でビルドできない。`flequit-core` →
  `flequit-infrastructure` → `sea-orm` + `sqlx-sqlite` を引くため。よって
  `flequit-web` は `flequit-ui` に依存せず、同じ `.slint` を再コンパイルする
- `slint-build` の翻訳ドメインは `CARGO_PKG_NAME` 固定で上書きできないため、
  `build.rs` が `i18n/**/flequit-ui.po` を `OUT_DIR` に `flequit-web.po` として
  ステージングしてから渡している
- `#[wasm_bindgen(start)]` はデスクトップでも通る。おかげで wasm ツールチェーン
  なしでも `cargo test -p flequit-web` でサンプルデータを検証できる
  （`cfg(target_arch)` を使わずに済む点でも都合が良い）
- ブラウザ内永続化を捨てたことで、wasm で動かす必要があるのは
  「UI + HTTP クライアント」だけになる。`sea-orm` を wasm 対応させる話が消え、
  Web 対応の重さがモバイルより軽くなった

---

## 8. 保存先の選択（将来設計・未着手）

> 保存先の構成と層の割り当ては 2026-09-08 に確定し、
> `docs/ja/develop/design/data/storage-targets.md` に起こした。
> API・スキーマ・認証・同期プロトコルは引き続き未設計。

保存先は**ユーザーが選ぶ**。アプリの種別が保存先を決めるのではない。

| 保存先 | Desktop | Mobile | Web | 想定 |
| --- | --- | --- | --- | --- |
| ローカル | ○ | ○ | × | SQLite + Automerge。現在の実装 |
| クラウドストレージ | ○ | ○ | × | ユーザー所有の同期フォルダ等に Automerge ドキュメントを置く |
| バックエンドサーバ | ○ | ○ | ○ | Flequit が用意するサーバ。Web 版の唯一の保存先 |

- デスクトップ / モバイルは**ローカル専用ではない**。ローカル保存に加えて
  バックエンドサーバと接続してやりとりできるようにする
- Web 版はバックエンドサーバ専用。ブラウザ内には保存しない
- **ローカルが常に正**、クラウドストレージとバックエンドサーバは同期先。
  併用できる（2026-09-08 決定）。Web だけは例外でサーバが正

### 決めるべきこと

- [x] 複数保存先の同時利用可否と、その場合の正となる保存先（2026-09-08）
- [x] 保存先の切り替え時に既存データをどうするか（移行 / 併存 / 破棄）
      → ローカルが常に残るため、切り替えは同期先の追加・削除になり移行は起きない
- [ ] 認証方式とアカウントの扱い（`Account` モデルと `load_current_account` は既にある）
- [ ] 同期の粒度と競合解決（Automerge をそのまま転送するのか、API を切るのか）
- [x] オフライン時の書き込みをどう扱うか（キュー / ローカルへフォールバック）
      → ローカルが正なので書き込みは常に成功し、キューは要らない
- [x] ローカルのSQLite / Automergeの片方だけが失敗したときの扱い（2026-09-12）
      → 両方の成功を保存成功とし、既知の失敗はrollback、クラッシュ時は
        SQLiteの操作ジャーナルから再開または補償する
- [ ] 同期の起動契機（起動時 / 変更時 / 定期 / 手動）と失敗時の再試行
- [ ] 複数の同期先があるときの順序と、片方だけ失敗したときの扱い
- [ ] サーバ側の実装言語とホスティング

### 実装作業（設計確定後）

2026-09-13に第1段階としてタスク更新経路へ着手した。Runtime Storeの
revision付きPending Mutation、同一タスクの保存直列化、SQLiteのentity revisionと
操作ジャーナル、SQLite + Automergeのtransaction port、起動時のprepared復旧を
実装済み。作成・削除・復元とタスク以外のエンティティへの展開は後続作業とする。

- [ ] Runtime Storeを実行時の唯一のエンティティ状態源にする
- [ ] エンティティrevisionと順序付きPending Mutationを実装する
- [ ] SQLite操作ジャーナルと起動時復旧を実装する
- [ ] 作成・更新・削除・復元をSQLite + Automergeのtransaction portへ統合する
- [ ] 途中失敗、連続変更、クラッシュ復旧のテストを追加する
- [ ] バックエンドサーバ本体（別リポジトリになる可能性あり）
- [ ] `flequit-infrastructure-remote`。デスクトップ / モバイルでは**同期層**として
      ローカルの後ろに置き、Web では repository trait の実装そのものになる。
      いずれも `flequit-core` から上は無変更で済ませる
- [ ] 保存先の選択 UI と永続化（`flequit-settings`）
- [ ] Web 版のバックエンド API クライアント（7. の残項目）
- [ ] 接続状態・同期状態の UI 表示
- [ ] `Capability::BackgroundSync` を実際の同期に接続する

#### 方針を決めた経緯（2026-09-08）

- 正となる保存先は**ローカル**に決めた。ネットワークに関係なく書き込みが成功し、
  複製同士のマージは Automerge の CRDT がそのまま担い、保存先の切り替えが
  データ移行にならない。詳細と層構成は
  `docs/ja/develop/design/data/storage-targets.md`
- その結果、サーバが repository trait の実装になるのは Web だけになった。
  デスクトップ / モバイルでは読み取り経路に入らない

- 「ローカル / クラウドストレージ / Web のどこに保存するもユーザーの自由」という
  前提をユーザーから確認した。これにより 2. の「Web 版は同期サーバ設計の確定後」
  という保留が、「Web だけの話ではなく全プラットフォーム共通の保存先設計」に変わった
- 保存先を差し替えるのは repository trait の裏側なので、レイヤ構造は既に対応済み。
  `flequit-core` 以上を触らずに追加できる想定
- 2026-09-12に、アプリ実行中はRuntime Storeを唯一の状態源とし、
  revision付きPending Mutationをエンティティ単位で直列化する方針を決定した。
  ローカル二層保存のクラッシュ復旧にはSQLiteの永続操作ジャーナルを使用する。
  詳細は`docs/ja/develop/design/data/runtime-store-and-mutations.md`

---

## 9. P4 — 配布・将来

- [ ] `cargo-packager` の設定と各 OS インストーラ生成
- [ ] コード署名（Windows / macOS notarization / Android / iOS）
- [x] `cargo audit` の CI 組み込み（2026-09-08）
- [ ] 自動アップデート（`Capability::SelfUpdate`）
- [ ] システムトレイ / グローバルショートカット（デスクトップのみ）
- [ ] Automerge によるクラウド同期（roadmap ver1.2 以降）— 8. と併せて設計する

---

#### 実装時の判断（2026-09-08）

- `.github/workflows/audit.yml` を新設し、push / PR に加えて毎週月曜にも走らせる。
  新しい advisory は変更が無くても増えるため
- 脆弱性で失敗、unmaintained は警告のまま。無関係な PR を止めないため
- 除外は `.cargo/audit.toml` に理由つきで書く。現時点の 1 件は rsa 0.9.10
  （RUSTSEC-2023-0071、修正版なし）。sqlx-mysql 経由でしか到達せず、
  このワークスペースは `sqlx-sqlite` しか有効にしていないためビルドされない
  （`cargo tree -i sqlx-mysql --target all` が空）。Cargo.lock は
  フィーチャに関係なく解決されるので、lock にだけ現れる

---

## 10. 環境メモ

- Linux 実行には `libxkbcommon-x11-0` が必要
- テスト前に `./scripts/test-prepare.sh`（SQLite テンプレート DB を 1 度だけ作成）
- Windows の開発機では `cargo test` の際に `CARGO_INCREMENTAL=0` を推奨。
  インクリメンタルディレクトリの書き込みが拒否されて rustc が
  `STATUS_STACK_BUFFER_OVERRUN` で落ちることがある
- `cargo test --workspace` で、リンクし直したばかりのテスト実行ファイルが
  Windows のアプリケーション制御ポリシーにブロックされることがある
  （`os error 4551`、doctest の場合は出力が空のまま `doctest failed` だけが出る）。
  対象が実行のたびに変わるのでコードの問題ではない。同じコマンドを
  もう一度流すか、`-p <crate> --doc` のように絞って再実行すれば通る
- 開発機の C ドライブの空きは 2026-09-08 時点で 11 GB
  （同日昼の時点では 0.7 GB しかなく `rustup target add` が失敗していた）。
  ターゲットの追加自体は容量的に可能になったが、Android SDK/NDK と Xcode は
  未導入のままで、モバイル / Web の実ビルド検証は引き続き CI 側でのみ可能
