# プラットフォーム抽象化設計

Flequit は Windows / macOS / Linux / Android / iOS を単一コードベースで対応する。
OS ごとに可否と実装が異なる機能は **`flequit-platform` クレート** に隔離し、
他のクレートからは条件コンパイルを排除する。

> 実装の正本は `crates/flequit-platform/` を参照。

## 設計原則

1. **`#[cfg(target_os = ...)]` は `flequit-platform` の内部にのみ書く**
   - UI・ViewModel・core・infrastructure に条件コンパイルを漏らさない
   - CI で `flequit-ui` / `flequit-core` 配下の `cfg(target_os)` を検出して失敗させる
2. **未対応機能はエラーではなく Capability で表現する**
   - 「呼んでみたら失敗した」ではなく「呼ぶ前に対応可否がわかる」形にする
   - UI は非対応機能のメニュー項目自体を表示しない
3. **インターフェースは全プラットフォーム共通**
   - 呼び出し側のコードがプラットフォームで分岐しないこと
4. **テスト可能にする**
   - trait として定義し、テストではモック実装を注入できるようにする

## モジュール構成

```text
crates/flequit-platform/src/
├── lib.rs
├── capability.rs        # Capability の定義と問い合わせ
├── paths.rs             # ディレクトリ解決
├── notification.rs      # ローカル通知
├── file_dialog.rs       # ファイル選択
├── opener.rs            # 外部アプリ起動
├── lifecycle.rs         # サスペンド/レジュームの購読
├── error.rs
└── platform/
    ├── mod.rs           # cfg による実装の切替（ここだけが cfg を持つ）
    ├── desktop/         # Windows / macOS / Linux
    ├── android/
    └── ios/
```

## Capability

実行時に機能の可否を問い合わせる。UI はこれを見て項目を出し分ける。

| Capability | Desktop | Android | iOS | 用途 |
| --- | --- | --- | --- | --- |
| `SystemTray` | ✅ | ❌ | ❌ | トレイ常駐・クイック追加 |
| `GlobalHotkey` | ✅ | ❌ | ❌ | グローバルショートカット |
| `MultiWindow` | ✅ | ❌ | ❌ | 別ウィンドウでのタスク詳細 |
| `OsTrash` | ✅ | ❌ | ❌ | 完全削除時の OS ゴミ箱への移動 |
| `ArbitraryFilePath` | ✅ | ❌ | ❌ | 任意パスへのエクスポート |
| `LocalNotification` | ✅ | ✅ | ✅ | リマインダー |
| `FilePicker` | ✅ | ✅ | ✅ | インポート/エクスポート、アイコン選択 |
| `BackgroundSync` | ✅ | ⚠️ 制限あり | ⚠️ 制限あり | バックグラウンド同期 |

`⚠️` はプラットフォームの制約下でのみ動作することを示す。
モバイルの OS はバックグラウンド実行を任意のタイミングで打ち切るため、
同期処理は中断されうる前提で設計する。

### 問い合わせ

`Platform::capabilities()` が `Capabilities` を返し、
UI は `capabilities.has(Capability::SystemTray)` の形で判定する。
判定結果は起動時に Slint の global へ流し込み、`.slint` 側では
`Capabilities.system-tray` のような `bool` プロパティとして参照する。

## ディレクトリ解決

すべてのファイル I/O は `flequit-platform::paths` が返すパスを起点にする。
パスをハードコードしてはならない。

| 種別 | 用途 | Desktop | Android | iOS |
| --- | --- | --- | --- | --- |
| `data_dir()` | SQLite DB、Automerge ファイル | `directories` の data local dir | アプリ内部ストレージ | `Application Support` |
| `config_dir()` | 設定ファイル | `directories` の config dir | 内部ストレージ配下 `config/` | `Application Support/config` |
| `cache_dir()` | 一時ファイル | `directories` の cache dir | `getCacheDir()` | `Caches` |
| `log_dir()` | ログファイル | data dir 配下 `logs/` | 内部ストレージ配下 `logs/` | `Caches/logs` |

モバイルではすべてアプリサンドボックス内に配置し、外部ストレージを既定で使わない。

## ローカル通知

リマインダー機能で使用する。

| 項目 | 内容 |
| --- | --- |
| インターフェース | `notify(request: NotificationRequest) -> Result<NotificationId>` |
| Desktop | `notify-rust` |
| Android | 通知チャネル + `NotificationManager` |
| iOS | `UNUserNotificationCenter` |

- モバイルは **実行時に通知許可を要求** する必要がある。
  `request_permission()` を用意し、初回リマインダー設定時に呼ぶ
- 許可されなかった場合は設定画面に理由と再要求導線を表示する
- 予約通知（将来時刻の通知）は OS のスケジューラに登録する。
  アプリのプロセスが生存していることを前提にしない

## ファイル選択

| 項目 | 内容 |
| --- | --- |
| インターフェース | `pick_file(filter) -> Option<FileHandle>` / `save_file(suggested_name) -> Option<FileHandle>` |
| Desktop | `rfd` |
| Android | Storage Access Framework（`ACTION_OPEN_DOCUMENT`） |
| iOS | `UIDocumentPickerViewController` |

**重要**: モバイルでは選択結果が「パス」ではなく「URI / セキュリティスコープ付きハンドル」になる。
そのため戻り値は `PathBuf` ではなく `FileHandle` 抽象とし、
読み書きは `FileHandle::read()` / `FileHandle::write()` を通す。

## 外部アプリ起動

| 項目 | 内容 |
| --- | --- |
| インターフェース | `open_url(url)` / `open_path(path)` |
| Desktop | `opener` |
| Android | `Intent.ACTION_VIEW` |
| iOS | `UIApplication.open` |

## ライフサイクル

`lifecycle` モジュールがサスペンド/レジュームを購読可能にする。

| イベント | Desktop | Android | iOS |
| --- | --- | --- | --- |
| `Suspend` | ウィンドウ最小化・非アクティブ | `onPause` | `applicationDidEnterBackground` |
| `Resume` | ウィンドウ復帰 | `onResume` | `applicationWillEnterForeground` |
| `LowMemory` | 発生しない | `onTrimMemory` | `didReceiveMemoryWarning` |

アプリ側の対応:

- `Suspend`: 未保存の変更をフラッシュし、同期タスクとタイマーを停止する
- `Resume`: データを再読込し、同期を再開する
- `LowMemory`: 非アクティブなプロジェクトの Automerge ドキュメントを解放する

**終了イベントに依存した保存処理を書いてはならない。**
モバイルの OS は予告なくプロセスを破棄する。

## 削除ファイルの扱い

`design/data/automerge-repo-dataflow.md` §6 の `.deleted/` フォルダ方式は
全プラットフォーム共通で動作する。その先の「完全削除」のみが分岐する。

| プラットフォーム | 完全削除の実装 |
| --- | --- |
| Desktop | `trash` クレートで OS ゴミ箱へ移動（`Capability::OsTrash`） |
| Android / iOS | ファイルを即時削除（OS ゴミ箱の概念がないため） |

## エラー処理

```text
PlatformError
├── Unsupported          # そのプラットフォームで機能自体が存在しない
├── PermissionDenied     # ユーザーが許可しなかった
├── Cancelled            # ユーザーがダイアログをキャンセルした
└── Io(std::io::Error)
```

- `Unsupported` は「Capability を確認せずに呼んだ」というプログラミングエラーを示す。
  正常系で発生させない
- `Cancelled` はエラー表示せず、単に何もしない
- `PermissionDenied` はユーザーへ理由と再要求導線を提示する

## テスト方針

- `flequit-platform` は trait ベースで定義し、`MockPlatform` をテストで注入する
- ディレクトリ解決は `tempfile` を使ったテスト用実装に差し替える
- 各プラットフォーム固有実装は CI のクロスコンパイルでビルド確認する
  （実行はデスクトップと Android エミュレータ）

## 関連ドキュメント

- [全体アーキテクチャ](../architecture.md)
- [技術スタック](../tech-stack.md)
- [レスポンシブレイアウト](../ui/responsive-layout.md)
- [Automerge データフロー](../data/automerge-repo-dataflow.md)
- [デプロイメント](../deployment.md)
