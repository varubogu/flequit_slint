//! Automerge ドキュメントと JSON の相互変換
//!
//! リポジトリはエンティティを serde で JSON にしてから Automerge に書き、読むときは逆を行う。

pub mod read;
pub mod reconcile;
