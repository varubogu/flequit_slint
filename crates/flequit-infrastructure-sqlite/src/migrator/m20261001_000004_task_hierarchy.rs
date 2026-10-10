//! タスクの階層化（リストなしのタスク・任意階層のサブタスク）
//!
//! - `tasks.list_id` を NULL 可にする（プロジェクト直下のタスク）
//! - `tasks.parent_task_id` を足し、サブタスクを「親を持つタスク」として `tasks` へ移す
//! - `subtask_tags` / `subtask_assignments` を `task_tags` / `task_assignments` へ移す
//! - `subtasks` / `subtask_tags` / `subtask_assignments` / `subtask_recurrence` を消す
//!
//! SQLite は列の NULL 制約や外部キーを `ALTER TABLE` で変えられないため `tasks` を作り直す。
//! 外部キーが有効なままの `DROP TABLE` は暗黙の DELETE で `ON DELETE CASCADE` を起こすので、
//! `tasks` を参照するテーブルの行を一時テーブルへ退避し、参照する側から先に消してから作り直す。
//! マイグレーションはプール上の接続で実行されるため、1 つのトランザクション（= 1 つの接続）で行う。
//!
//! 設計: `plans/plan.md` §3、`docs/ja/develop/design/data/entity/projects.md`

use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ConnectionTrait, TransactionTrait};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let txn = manager.get_connection().begin().await?;
        for statement in UP {
            txn.execute_unprepared(statement).await?;
        }
        txn.commit().await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // 2 段目より深いサブタスクとリストなしのタスクは元の形で表せない
        Err(DbErr::Migration(
            "m20261001_000004_task_hierarchy cannot be reverted".to_string(),
        ))
    }
}

/// 順に実行する文
///
/// 戻す行は親（タスク・タグ・ユーザー・繰り返しルール）が存在するものに限る。
/// 外部キー違反の行が 1 件でもあるとコミットに失敗し、アプリが起動できなくなるため。
const UP: &[&str] = &[
    // 途中の状態では外部キーを確かめず、コミット時にまとめて確かめる
    "PRAGMA defer_foreign_keys = ON",
    // 1. 退避
    "CREATE TEMP TABLE migration_tasks AS SELECT * FROM tasks",
    "CREATE TEMP TABLE migration_subtasks AS SELECT * FROM subtasks",
    "CREATE TEMP TABLE migration_task_tags AS SELECT * FROM task_tags",
    "CREATE TEMP TABLE migration_subtask_tags AS SELECT * FROM subtask_tags",
    "CREATE TEMP TABLE migration_task_assignments AS SELECT * FROM task_assignments",
    "CREATE TEMP TABLE migration_subtask_assignments AS SELECT * FROM subtask_assignments",
    "CREATE TEMP TABLE migration_task_recurrence AS SELECT * FROM task_recurrence",
    // 2. 参照する側から消す
    "DROP TABLE subtask_recurrence",
    "DROP TABLE subtask_tags",
    "DROP TABLE subtask_assignments",
    "DROP TABLE subtasks",
    "DROP TABLE task_recurrence",
    "DROP TABLE task_tags",
    "DROP TABLE task_assignments",
    "DROP INDEX IF EXISTS idx_tasks_end_date_status",
    "DROP TABLE tasks",
    // 3. 作り直す
    r#"
    CREATE TABLE tasks (
        project_id VARCHAR NOT NULL,
        id VARCHAR NOT NULL,
        list_id VARCHAR,
        parent_task_id VARCHAR,
        title VARCHAR NOT NULL,
        description VARCHAR,
        status VARCHAR NOT NULL,
        priority INTEGER NOT NULL DEFAULT 0,
        start_date TIMESTAMP,
        end_date TIMESTAMP,
        is_range_date BOOLEAN,
        reminders TEXT NOT NULL DEFAULT '[]',
        order_index INTEGER NOT NULL,
        is_archived BOOLEAN NOT NULL DEFAULT FALSE,
        created_at TIMESTAMP NOT NULL,
        updated_at TIMESTAMP NOT NULL,
        deleted BOOLEAN NOT NULL DEFAULT FALSE,
        updated_by VARCHAR NOT NULL,
        CONSTRAINT pk_tasks PRIMARY KEY (project_id, id),
        FOREIGN KEY (project_id) REFERENCES projects (id) ON DELETE CASCADE,
        FOREIGN KEY (project_id, list_id) REFERENCES task_lists (project_id, id) ON DELETE CASCADE,
        FOREIGN KEY (project_id, parent_task_id) REFERENCES tasks (project_id, id) ON DELETE CASCADE
    )
    "#,
    "CREATE INDEX IF NOT EXISTS idx_tasks_end_date_status ON tasks(end_date, status)",
    "CREATE INDEX IF NOT EXISTS idx_tasks_project_list ON tasks(project_id, list_id)",
    "CREATE INDEX IF NOT EXISTS idx_tasks_project_parent ON tasks(project_id, parent_task_id)",
    r#"
    CREATE TABLE task_recurrence (
        project_id VARCHAR NOT NULL,
        task_id VARCHAR NOT NULL,
        recurrence_rule_id VARCHAR NOT NULL,
        created_at TIMESTAMP NOT NULL,
        updated_at TIMESTAMP NOT NULL,
        deleted BOOLEAN NOT NULL DEFAULT FALSE,
        updated_by VARCHAR NOT NULL,
        CONSTRAINT pk_task_recurrence PRIMARY KEY (project_id, task_id, recurrence_rule_id),
        FOREIGN KEY (project_id, task_id) REFERENCES tasks (project_id, id) ON DELETE CASCADE,
        FOREIGN KEY (project_id, recurrence_rule_id) REFERENCES recurrence_rules (project_id, id) ON DELETE CASCADE
    )
    "#,
    r#"
    CREATE TABLE task_tags (
        project_id VARCHAR NOT NULL,
        task_id VARCHAR NOT NULL,
        tag_id VARCHAR NOT NULL,
        created_at TIMESTAMP NOT NULL,
        updated_at TIMESTAMP NOT NULL,
        deleted BOOLEAN NOT NULL DEFAULT FALSE,
        updated_by VARCHAR NOT NULL,
        CONSTRAINT pk_task_tags PRIMARY KEY (project_id, task_id, tag_id),
        FOREIGN KEY (project_id, task_id) REFERENCES tasks (project_id, id) ON DELETE CASCADE,
        FOREIGN KEY (project_id, tag_id) REFERENCES tags (project_id, id) ON DELETE CASCADE
    )
    "#,
    r#"
    CREATE TABLE task_assignments (
        project_id VARCHAR NOT NULL,
        task_id VARCHAR NOT NULL,
        user_id VARCHAR NOT NULL,
        created_at TIMESTAMP NOT NULL,
        updated_at TIMESTAMP NOT NULL,
        deleted BOOLEAN NOT NULL DEFAULT FALSE,
        updated_by VARCHAR NOT NULL,
        CONSTRAINT pk_task_assignments PRIMARY KEY (project_id, task_id, user_id),
        FOREIGN KEY (project_id, task_id) REFERENCES tasks (project_id, id) ON DELETE CASCADE,
        FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
    )
    "#,
    // 4. 戻す。タスク（すべて最上位）→ サブタスク（親を持つタスク）の順
    r#"
    INSERT INTO tasks (
        project_id, id, list_id, parent_task_id, title, description, status, priority,
        start_date, end_date, is_range_date, reminders, order_index, is_archived,
        created_at, updated_at, deleted, updated_by
    )
    SELECT
        t.project_id, t.id, t.list_id, NULL, t.title, t.description, t.status, t.priority,
        t.start_date, t.end_date, t.is_range_date, t.reminders, t.order_index, t.is_archived,
        t.created_at, t.updated_at, t.deleted, t.updated_by
    FROM migration_tasks AS t
    WHERE EXISTS (SELECT 1 FROM projects AS p WHERE p.id = t.project_id)
      AND EXISTS (
        SELECT 1 FROM task_lists AS l WHERE l.project_id = t.project_id AND l.id = t.list_id
      )
    "#,
    // 旧 `completed` は状態より前からある項目で、立っていれば完了として扱っていた
    r#"
    INSERT OR IGNORE INTO tasks (
        project_id, id, list_id, parent_task_id, title, description, status, priority,
        start_date, end_date, is_range_date, reminders, order_index, is_archived,
        created_at, updated_at, deleted, updated_by
    )
    SELECT
        s.project_id, s.id, NULL, s.task_id, s.title, s.description,
        CASE WHEN s.completed THEN 'completed' ELSE s.status END,
        COALESCE(s.priority, 0),
        s.plan_start_date, s.plan_end_date, s.is_range_date, '[]', s.order_index, FALSE,
        s.created_at, s.updated_at, s.deleted, s.updated_by
    FROM migration_subtasks AS s
    WHERE EXISTS (
        SELECT 1 FROM tasks AS t WHERE t.project_id = s.project_id AND t.id = s.task_id
    )
    "#,
    r#"
    INSERT OR IGNORE INTO task_tags
        (project_id, task_id, tag_id, created_at, updated_at, deleted, updated_by)
    SELECT r.project_id, r.task_id, r.tag_id, r.created_at, r.updated_at, r.deleted, r.updated_by
    FROM migration_task_tags AS r
    WHERE EXISTS (SELECT 1 FROM tasks AS t WHERE t.project_id = r.project_id AND t.id = r.task_id)
      AND EXISTS (SELECT 1 FROM tags AS g WHERE g.project_id = r.project_id AND g.id = r.tag_id)
    "#,
    r#"
    INSERT OR IGNORE INTO task_tags
        (project_id, task_id, tag_id, created_at, updated_at, deleted, updated_by)
    SELECT r.project_id, r.subtask_id, r.tag_id, r.created_at, r.updated_at, r.deleted, r.updated_by
    FROM migration_subtask_tags AS r
    WHERE EXISTS (
        SELECT 1 FROM tasks AS t WHERE t.project_id = r.project_id AND t.id = r.subtask_id
    )
      AND EXISTS (SELECT 1 FROM tags AS g WHERE g.project_id = r.project_id AND g.id = r.tag_id)
    "#,
    r#"
    INSERT OR IGNORE INTO task_assignments
        (project_id, task_id, user_id, created_at, updated_at, deleted, updated_by)
    SELECT r.project_id, r.task_id, r.user_id, r.created_at, r.updated_at, r.deleted, r.updated_by
    FROM migration_task_assignments AS r
    WHERE EXISTS (SELECT 1 FROM tasks AS t WHERE t.project_id = r.project_id AND t.id = r.task_id)
      AND EXISTS (SELECT 1 FROM users AS u WHERE u.id = r.user_id)
    "#,
    r#"
    INSERT OR IGNORE INTO task_assignments
        (project_id, task_id, user_id, created_at, updated_at, deleted, updated_by)
    SELECT r.project_id, r.subtask_id, r.user_id, r.created_at, r.updated_at, r.deleted, r.updated_by
    FROM migration_subtask_assignments AS r
    WHERE EXISTS (
        SELECT 1 FROM tasks AS t WHERE t.project_id = r.project_id AND t.id = r.subtask_id
    )
      AND EXISTS (SELECT 1 FROM users AS u WHERE u.id = r.user_id)
    "#,
    r#"
    INSERT OR IGNORE INTO task_recurrence
        (project_id, task_id, recurrence_rule_id, created_at, updated_at, deleted, updated_by)
    SELECT r.project_id, r.task_id, r.recurrence_rule_id, r.created_at, r.updated_at, r.deleted,
           r.updated_by
    FROM migration_task_recurrence AS r
    WHERE EXISTS (SELECT 1 FROM tasks AS t WHERE t.project_id = r.project_id AND t.id = r.task_id)
      AND EXISTS (
        SELECT 1 FROM recurrence_rules AS g
        WHERE g.project_id = r.project_id AND g.id = r.recurrence_rule_id
      )
    "#,
    // 5. 片付け（旧サブタスクの繰り返しの関連は UI から書いていなかったため移さない）
    "DROP TABLE migration_tasks",
    "DROP TABLE migration_subtasks",
    "DROP TABLE migration_task_tags",
    "DROP TABLE migration_subtask_tags",
    "DROP TABLE migration_task_assignments",
    "DROP TABLE migration_subtask_assignments",
    "DROP TABLE migration_task_recurrence",
];

#[cfg(test)]
mod tests {
    use sea_orm_migration::MigratorTrait;
    use sea_orm_migration::sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

    use crate::migrator::Migrator;

    /// マイグレーション前の形のデータを入れてから、このマイグレーションを当てる
    #[tokio::test]
    async fn subtasks_become_child_tasks_with_their_tags_and_assignees() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("PRAGMA foreign_keys = ON")
            .await
            .unwrap();
        // 直前（000003）まで当てる
        Migrator::up(&db, Some(3)).await.unwrap();

        let now = "2026-09-30T00:00:00Z";
        let statements = [
            format!(
                "INSERT INTO users (id, handle_id, display_name, is_active, created_at, updated_at, updated_by) \
                 VALUES ('u1', 'u1', 'User', TRUE, '{now}', '{now}', 'u1')"
            ),
            format!(
                "INSERT INTO projects (id, name, order_index, is_archived, created_at, updated_at, updated_by) \
                 VALUES ('p1', 'Project', 0, FALSE, '{now}', '{now}', 'u1')"
            ),
            format!(
                "INSERT INTO task_lists (project_id, id, name, order_index, created_at, updated_at, updated_by) \
                 VALUES ('p1', 'l1', 'List', 0, '{now}', '{now}', 'u1')"
            ),
            format!(
                "INSERT INTO tags (project_id, id, name, created_at, updated_at, updated_by) \
                 VALUES ('p1', 'g1', 'Tag', '{now}', '{now}', 'u1')"
            ),
            format!(
                "INSERT INTO tasks (project_id, id, list_id, title, status, priority, order_index, created_at, updated_at, updated_by) \
                 VALUES ('p1', 't1', 'l1', 'Task', 'not_started', 2, 0, '{now}', '{now}', 'u1')"
            ),
            format!(
                "INSERT INTO task_tags (project_id, task_id, tag_id, created_at, updated_at, updated_by) \
                 VALUES ('p1', 't1', 'g1', '{now}', '{now}', 'u1')"
            ),
            format!(
                "INSERT INTO subtasks (project_id, id, task_id, title, status, priority, order_index, completed, created_at, updated_at, updated_by) \
                 VALUES ('p1', 's1', 't1', 'Done by flag', 'not_started', NULL, 1, TRUE, '{now}', '{now}', 'u1')"
            ),
            format!(
                "INSERT INTO subtasks (project_id, id, task_id, title, status, priority, order_index, completed, plan_end_date, created_at, updated_at, updated_by) \
                 VALUES ('p1', 's2', 't1', 'Open', 'in_progress', 3, 2, FALSE, '{now}', '{now}', '{now}', 'u1')"
            ),
            format!(
                "INSERT INTO subtask_tags (project_id, subtask_id, tag_id, created_at, updated_at, updated_by) \
                 VALUES ('p1', 's2', 'g1', '{now}', '{now}', 'u1')"
            ),
            format!(
                "INSERT INTO subtask_assignments (project_id, subtask_id, user_id, created_at, updated_at, updated_by) \
                 VALUES ('p1', 's2', 'u1', '{now}', '{now}', 'u1')"
            ),
        ];
        for statement in &statements {
            db.execute_unprepared(statement).await.unwrap();
        }

        Migrator::up(&db, None).await.unwrap();

        let rows = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT id, list_id, parent_task_id, status, priority, end_date FROM tasks ORDER BY id",
            ))
            .await
            .unwrap();
        let read = |index: usize, column: &str| -> Option<String> {
            rows[index].try_get::<Option<String>>("", column).unwrap()
        };
        assert_eq!(rows.len(), 3);
        // s1: 旧 completed で完了、優先度 NULL は 0
        assert_eq!(read(0, "id").as_deref(), Some("s1"));
        assert_eq!(read(0, "list_id"), None);
        assert_eq!(read(0, "parent_task_id").as_deref(), Some("t1"));
        assert_eq!(read(0, "status").as_deref(), Some("completed"));
        assert_eq!(rows[0].try_get::<i32>("", "priority").unwrap(), 0);
        // s2: 予定終了日時は期限（end_date）へ
        assert_eq!(read(1, "id").as_deref(), Some("s2"));
        assert_eq!(read(1, "status").as_deref(), Some("in_progress"));
        assert_eq!(read(1, "end_date").as_deref(), Some(now));
        // t1: 最上位のまま
        assert_eq!(read(2, "id").as_deref(), Some("t1"));
        assert_eq!(read(2, "list_id").as_deref(), Some("l1"));
        assert_eq!(read(2, "parent_task_id"), None);

        let count = |sql: &str| {
            let db = &db;
            let sql = sql.to_string();
            async move {
                db.query_one_raw(Statement::from_string(DbBackend::Sqlite, sql))
                    .await
                    .unwrap()
                    .unwrap()
                    .try_get::<i64>("", "n")
                    .unwrap()
            }
        };
        assert_eq!(
            count("SELECT COUNT(*) AS n FROM task_tags WHERE task_id IN ('t1', 's2')").await,
            2
        );
        assert_eq!(
            count("SELECT COUNT(*) AS n FROM task_assignments WHERE task_id = 's2'").await,
            1
        );
        assert_eq!(
            count("SELECT COUNT(*) AS n FROM sqlite_master WHERE name LIKE 'subtask%'").await,
            0
        );

        // 親を消すと子も消える
        db.execute_unprepared("DELETE FROM tasks WHERE id = 't1'")
            .await
            .unwrap();
        assert_eq!(count("SELECT COUNT(*) AS n FROM tasks").await, 0);
        assert_eq!(count("SELECT COUNT(*) AS n FROM task_tags").await, 0);
    }

    #[tokio::test]
    async fn a_task_without_a_list_belongs_to_the_project() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("PRAGMA foreign_keys = ON")
            .await
            .unwrap();
        Migrator::up(&db, None).await.unwrap();

        let now = "2026-10-01T00:00:00Z";
        db.execute_unprepared(&format!(
            "INSERT INTO projects (id, name, order_index, is_archived, created_at, updated_at, updated_by) \
             VALUES ('p1', 'Project', 0, FALSE, '{now}', '{now}', 'u1')"
        ))
        .await
        .unwrap();
        db.execute_unprepared(&format!(
            "INSERT INTO tasks (project_id, id, list_id, title, status, order_index, created_at, updated_at, updated_by) \
             VALUES ('p1', 't1', NULL, 'Loose', 'not_started', 0, '{now}', '{now}', 'u1')"
        ))
        .await
        .unwrap();

        // プロジェクトを消すとリストを経由せずに消える
        db.execute_unprepared("DELETE FROM projects WHERE id = 'p1'")
            .await
            .unwrap();
        let left = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n FROM tasks",
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<i64>("", "n")
            .unwrap();
        assert_eq!(left, 0);
    }
}
