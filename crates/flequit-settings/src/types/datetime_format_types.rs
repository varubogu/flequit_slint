//! 日付と時刻の表示フォーマットに関連する型を定義します。
use serde::{Deserialize, Serialize};

/// 日付時刻のフォーマットグループを示します。
///
/// 旧 Tauri 版の `settings.yml` と互換性を保つため snake_case で読み書きし、
/// バリアント名そのままの表記も alias で受け付ける。
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DateTimeFormatGroup {
    /// デフォルト設定
    #[serde(alias = "Default")]
    #[default]
    Default,
    /// プリセット選択
    #[serde(alias = "Preset")]
    Preset,
    /// カスタム設定
    #[serde(alias = "Custom")]
    Custom,
    /// カスタム書式文字列
    #[serde(alias = "CustomFormat")]
    CustomFormat,
}
