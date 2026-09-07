//! アプリケーション設定構造体
//!
//! このモジュールはアプリケーションの全設定項目を管理する構造体を定義します。

use partially::Partial;
use serde::{Deserialize, Serialize};

use super::custom_due_filter::CustomDueFilter;
use super::datetime_format::DateTimeFormat;
use super::due_date_buttons::DueDateButtons;
use super::recurrence_preset::RecurrencePreset;
use super::time_label::TimeLabel;
use super::view_item::ViewItem;

/// アプリケーション設定構造体（フラット構造）
///
/// アプリケーションの全設定項目を単一の構造体で管理します。
/// フロントエンドのSettings型に対応しています。
/// 設定ファイルは旧 Tauri 版と同じ `settings.yml` を共有するため、
/// キーは旧実装と同じ camelCase で読み書きし、snake_case も alias で受け付ける。
#[derive(Debug, Clone, Serialize, Deserialize, Partial)]
#[serde(rename_all = "camelCase")]
#[partially(derive(Debug, Clone, Serialize, Deserialize, Default))]
pub struct Settings {
    // テーマ・外観設定
    /// UIテーマ（"system", "light", "dark"）
    pub theme: String,
    /// 言語設定（ISO 639-1形式）
    pub language: String,
    /// フォント名
    pub font: String,
    /// フォントサイズ
    #[serde(alias = "font_size")]
    pub font_size: i32,
    /// フォント色
    #[serde(alias = "font_color")]
    pub font_color: String,
    /// 背景色
    #[serde(alias = "background_color")]
    pub background_color: String,

    // 基本設定
    /// 週の開始曜日（"sunday", "monday"）
    #[serde(alias = "week_start")]
    pub week_start: String,
    /// Enables Vim-style task-list navigation (j/k and g/G).
    #[serde(default, alias = "vim_mode")]
    pub vim_mode: bool,
    /// タイムゾーン
    pub timezone: String,
    /// カスタム期限フィルタ（値と単位。旧形式の日数配列も読み込める）
    #[serde(
        default,
        alias = "custom_due_filters",
        alias = "customDueDays",
        alias = "custom_due_days"
    )]
    pub custom_due_filters: Vec<CustomDueFilter>,
    /// 繰り返し設定のカスタム項目
    #[serde(default, alias = "custom_recurrence_presets")]
    pub custom_recurrence_presets: Vec<RecurrencePreset>,
    /// 選択した日時フォーマット
    #[serde(alias = "datetime_format")]
    pub datetime_format: DateTimeFormat,
    /// 日時フォーマット一覧
    #[serde(alias = "datetime_formats")]
    pub datetime_formats: Vec<DateTimeFormat>,
    /// 時刻ラベル
    #[serde(alias = "time_labels")]
    pub time_labels: Vec<TimeLabel>,

    // 表示設定
    /// 期日ボタンの表示設定
    #[serde(alias = "due_date_buttons")]
    pub due_date_buttons: Vec<DueDateButtons>,
    /// ビューアイテム設定
    #[serde(alias = "view_items")]
    pub view_items: Vec<ViewItem>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".to_string(),
            language: "ja".to_string(),
            font: "system".to_string(),
            font_size: 13,
            font_color: "#000000".to_string(),
            background_color: "#FFFFFF".to_string(),
            week_start: "sunday".to_string(),
            vim_mode: false,
            timezone: "Asia/Tokyo".to_string(),
            custom_due_filters: vec![],
            custom_recurrence_presets: vec![],
            datetime_format: DateTimeFormat::default(),
            datetime_formats: vec![],
            time_labels: vec![],
            due_date_buttons: vec![
                DueDateButtons::new("overdue".into(), "overdue".into(), false, 0),
                DueDateButtons::new("today".into(), "today".into(), true, 1),
                DueDateButtons::new("tomorrow".into(), "tomorrow".into(), true, 2),
                DueDateButtons::new("three-days".into(), "three-days".into(), false, 3),
                DueDateButtons::new("this-week".into(), "this-week".into(), true, 4),
                DueDateButtons::new("this-month".into(), "this-month".into(), false, 5),
                DueDateButtons::new("this-quarter".into(), "this-quarter".into(), false, 6),
                DueDateButtons::new("this-year".into(), "this-year".into(), false, 7),
                DueDateButtons::new(
                    "this-fiscal-year".into(),
                    "this-fiscal-year".into(),
                    false,
                    8,
                ),
            ],
            view_items: vec![],
        }
    }
}
