# Web API 設計書

Flequit の同期サーバ (将来実装) の API 設計。現状はローカル完結 (SQLite + Automerge) で動作しており、本書は将来 Web 同期を実装する際の指針。

## 1. 基本設計

| 項目 | 内容 |
| --- | --- |
| 形式 | RESTful API |
| 通信 | JSON |
| プロトコル | HTTPS |
| 認証 | Bearer (JWT) |
| ベース URL | `https://api.flequit.com` |
| バージョニング | パスプレフィックス `/v1` |
| リソースパス | `/{resource}` |

### リクエストヘッダー

- `Authorization: Bearer {token}`
- `Content-Type: application/json`
- `Accept-Language: ja-JP` 等

### レスポンスフォーマット (共通)

`status` (`success` / `error`) + `data` (成功時) + `error` (`code` + `message`) の構造。

## 2. 認証 API

| エンドポイント | 説明 | 主リクエスト | 主レスポンス |
| --- | --- | --- | --- |
| `POST /v1/auth/login` | ログイン | `email`, `password` | `access_token`, `refresh_token`, `expires_in` |
| `POST /v1/auth/refresh` | トークン更新 | `refresh_token` | `access_token`, `expires_in` |

## 3. 同期 API

| エンドポイント | 説明 | 主リクエスト | 主レスポンス |
| --- | --- | --- | --- |
| `POST /v1/sync/tasks` | タスク同期 (差分送受信) | `last_sync_timestamp`, `changes[{ id, type, timestamp, data }]` (type は create/update/delete) | `sync_timestamp`, `changes[]`, `conflicts[{ id, server_version, client_version }]` |
| `GET /v1/sync/status` | 同期状態確認 | - | `last_sync_timestamp`, `pending_changes` |

## 4. エラー処理

### エラーコード体系

| カテゴリ | コード | 内容 |
| --- | --- | --- |
| 認証 | `AUTH_001` | 認証情報が無効 |
| 認証 | `AUTH_002` | トークンの有効期限切れ |
| 同期 | `SYNC_001` | 同期の競合 |
| 同期 | `SYNC_002` | 無効なタイムスタンプ |
| 一般 | `GENERAL_001` | サーバーエラー |
| 一般 | `GENERAL_002` | 無効なリクエスト |

### エラーレスポンス

`status: "error"` + `error: { code, message, details? }` の構造。

## 5. セキュリティ

### 認証・認可

- JWT ベース
- アクセストークン有効期限: **1 時間**
- リフレッシュトークン有効期限: **30 日**

一般的な HTTP API のセキュリティ対策（レート制限、アカウントロックアウト、
IP 監視）、パフォーマンス最適化（gzip、ETag、キャッシュヘッダー）、
OpenAPI によるドキュメント生成、セマンティックバージョニングによる変更管理は、
実装時に標準的な手法をそのまま採用する。本書では扱わない。
