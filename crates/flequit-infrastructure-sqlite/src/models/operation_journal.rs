use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "operation_journal")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub operation_id: String,
    pub entity_kind: String,
    pub project_id: String,
    pub entity_id: String,
    pub base_revision: i64,
    pub forward_data: String,
    pub rollback_data: String,
    pub status: String,
    pub recovery_file_path: Option<String>,
    pub recovery_file_checksum: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
