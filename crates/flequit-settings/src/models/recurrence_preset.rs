//! 繰り返し設定のカスタム項目モデル
//!
//! 繰り返しエディタが提供する既定の組み合わせ以外に、ユーザーがよく使う
//! 「単位 × 間隔」を登録しておくための設定。

use serde::{Deserialize, Serialize};

/// 繰り返しの単位
///
/// ドメインの `RecurrenceUnit` と同じ集合を持つ。設定クレートはドメインに
/// 依存しないため、同じ値をここでも定義する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecurrenceUnit {
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Quarter,
    HalfYear,
    Year,
}

/// ユーザーが登録した繰り返しパターン 1 件
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecurrencePreset {
    /// 間隔（1 以上）
    pub interval: i32,
    pub unit: RecurrenceUnit,
    /// ユーザーが付けた呼び名（「隔週」など）。空なら既定の表示名を使う。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

impl RecurrencePreset {
    pub fn new(interval: i32, unit: RecurrenceUnit) -> Self {
        Self::named(interval, unit, String::new())
    }

    pub fn named(interval: i32, unit: RecurrenceUnit, name: String) -> Self {
        Self {
            interval,
            unit,
            name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preset_round_trips_through_yaml() {
        let presets = vec![
            RecurrencePreset::new(2, RecurrenceUnit::Week),
            RecurrencePreset::named(6, RecurrenceUnit::Month, "半期".to_string()),
        ];

        let yaml = serde_yaml::to_string(&presets).unwrap();

        assert_eq!(
            serde_yaml::from_str::<Vec<RecurrencePreset>>(&yaml).unwrap(),
            presets
        );
    }

    #[test]
    fn a_half_year_unit_keeps_its_lowercase_name() {
        let yaml =
            serde_yaml::to_string(&RecurrencePreset::new(1, RecurrenceUnit::HalfYear)).unwrap();

        assert!(yaml.contains("halfyear"), "unexpected yaml: {yaml}");
    }
}
