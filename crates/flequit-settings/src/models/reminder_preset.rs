//! リマインダー候補のカスタム項目モデル

use serde::{Deserialize, Serialize};

/// リマインダーを基準日時よりどれだけ前に通知するかの単位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReminderUnit {
    Minute,
    Hour,
    Day,
}

impl ReminderUnit {
    /// 単位ごとの入力上限値。
    pub fn max_value(self) -> i32 {
        match self {
            Self::Minute => 43_200,
            Self::Hour => 8_760,
            Self::Day => 3_650,
        }
    }
}

/// ユーザーがリマインダー追加時に選べる相対時間 1 件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderPreset {
    pub value: i32,
    pub unit: ReminderUnit,
    /// ユーザーが付けた呼び名（「前日」など）。空なら既定の表示名を使う。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

impl ReminderPreset {
    pub fn new(value: i32, unit: ReminderUnit) -> Self {
        Self::named(value, unit, String::new())
    }

    pub fn named(value: i32, unit: ReminderUnit, name: String) -> Self {
        Self { value, unit, name }
    }
}

/// 設定ファイルに項目がない場合にも従来相当の候補を提供する。
pub fn default_reminder_presets() -> Vec<ReminderPreset> {
    vec![
        ReminderPreset::new(30, ReminderUnit::Minute),
        ReminderPreset::new(1, ReminderUnit::Hour),
        ReminderPreset::new(1, ReminderUnit::Day),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_round_trip_through_yaml() {
        let mut presets = default_reminder_presets();
        presets.push(ReminderPreset::named(
            1,
            ReminderUnit::Day,
            "前日".to_string(),
        ));
        let yaml = serde_yaml::to_string(&presets).unwrap();

        assert_eq!(
            serde_yaml::from_str::<Vec<ReminderPreset>>(&yaml).unwrap(),
            presets
        );
    }
}
