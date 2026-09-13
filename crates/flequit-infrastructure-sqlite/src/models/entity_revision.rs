use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "entity_revisions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub entity_kind: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub project_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub entity_id: String,
    pub revision: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
