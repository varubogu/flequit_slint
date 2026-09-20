//! Sea-ORM Migration
//!
//! マイグレーションファイルの管理と実行

use sea_orm_migration::prelude::*;

mod m20250101_000001_initial_schema;
mod m20260906_000002_task_reminders;
mod m20260920_000003_automerge_sync_queue;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20250101_000001_initial_schema::Migration),
            Box::new(m20260906_000002_task_reminders::Migration),
            Box::new(m20260920_000003_automerge_sync_queue::Migration),
        ]
    }
}
