//! カスタム期限フィルタ設定モデル

use serde::Deserializer;
use serde::{Deserialize, Serialize};

/// カスタム期限フィルタの単位
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CustomDueUnit {
    /// 分（「10分以内」）
    Minute,
    /// 時間（「1時間以内」）
    Hour,
    /// 日（「5日以内」）
    Day,
}

impl CustomDueUnit {
    /// 単位ごとの上限値。いずれも約1年を超えない範囲に収める。
    pub fn max_value(self) -> i32 {
        match self {
            Self::Minute => 43_200,
            Self::Hour => 8_760,
            Self::Day => 3_650,
        }
    }
}

/// ユーザーが追加した期限フィルタ 1 件
///
/// 旧形式（`custom_due_days: [1, 3, 7]`）の設定ファイルも読み込めるよう、
/// 整数値は「日」として解釈する。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CustomDueFilter {
    pub value: i32,
    pub unit: CustomDueUnit,
    /// ユーザーが付けた呼び名（「今期」など）。空なら既定の表示名を使う。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

impl CustomDueFilter {
    pub fn new(value: i32, unit: CustomDueUnit) -> Self {
        Self::named(value, unit, String::new())
    }

    pub fn named(value: i32, unit: CustomDueUnit, name: String) -> Self {
        Self { value, unit, name }
    }

    pub fn days(value: i32) -> Self {
        Self::new(value, CustomDueUnit::Day)
    }
}

impl<'de> Deserialize<'de> for CustomDueFilter {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            /// 単位が導入される前の形式。日数だけを並べていた。
            LegacyDays(i32),
            Full {
                value: i32,
                unit: CustomDueUnit,
                #[serde(default)]
                name: String,
            },
        }

        match Repr::deserialize(deserializer)? {
            Repr::LegacyDays(days) => Ok(Self::days(days)),
            Repr::Full { value, unit, name } => Ok(Self::named(value, unit, name)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_legacy_day_count_is_read_as_a_day_filter() {
        // Settings files written before units existed listed plain day counts.
        let filters: Vec<CustomDueFilter> = serde_yaml::from_str("- 1\n- 7\n").unwrap();

        assert_eq!(
            filters,
            [CustomDueFilter::days(1), CustomDueFilter::days(7)]
        );
    }

    #[test]
    fn a_filter_round_trips_through_yaml() {
        let filters = vec![
            CustomDueFilter::new(10, CustomDueUnit::Minute),
            CustomDueFilter::new(1, CustomDueUnit::Hour),
            CustomDueFilter::named(90, CustomDueUnit::Day, "今期".to_string()),
        ];

        let yaml = serde_yaml::to_string(&filters).unwrap();

        assert_eq!(
            serde_yaml::from_str::<Vec<CustomDueFilter>>(&yaml).unwrap(),
            filters
        );
    }

    #[test]
    fn an_unnamed_filter_writes_no_name_key() {
        let yaml = serde_yaml::to_string(&CustomDueFilter::days(5)).unwrap();

        assert!(!yaml.contains("name"), "unexpected yaml: {yaml}");
    }
}
