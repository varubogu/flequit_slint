//! 一時的な計測用。Automerge へのタスク保存 1 回あたりの時間を測る。
use std::time::Instant;

use chrono::Utc;
use flequit_infrastructure_automerge::infrastructure::task_projects::task::TaskLocalAutomergeRepository;
use flequit_model::models::task_projects::task::Task;
use flequit_model::types::id_types::{ProjectId, TaskId, TaskListId, UserId};
use flequit_model::types::task_types::TaskStatus;

fn task(project_id: ProjectId, list_id: TaskListId) -> Task {
    let now = Utc::now();
    Task {
        id: TaskId::new(),
        project_id,
        list_id,
        previous_task_id: None,
        title: "Water the plants".to_string(),
        description: Some("both balconies".to_string()),
        status: TaskStatus::NotStarted,
        priority: 3,
        plan_start_date: Some(now),
        plan_end_date: Some(now),
        do_start_date: None,
        do_end_date: None,
        is_range_date: Some(false),
        recurrence_rule: None,
        reminders: vec![now],
        order_index: 0,
        is_archived: false,
        assigned_user_ids: Vec::new(),
        tag_ids: Vec::new(),
        created_at: now,
        updated_at: now,
        deleted: false,
        updated_by: UserId::new(),
    }
}

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        // 実データのコピーに対する計測: 既存タスクのステータスを繰り返し更新する
        if let (Ok(path), Ok(project)) = (
            std::env::var("PROBE_DIR"),
            std::env::var("PROBE_PROJECT"),
        ) {
            let repo = TaskLocalAutomergeRepository::new(path.into()).await.unwrap();
            let project_id = ProjectId::from(project);
            let existing = repo.list_tasks(&project_id).await.unwrap();
            println!("real data: {} tasks", existing.len());
            let mut t = existing[0].clone();
            for round in 0..6 {
                t.status = if round % 2 == 0 {
                    TaskStatus::Completed
                } else {
                    TaskStatus::NotStarted
                };
                let started = Instant::now();
                repo.set_task(&project_id, &t).await.unwrap();
                println!(
                    "real round {round} set_task {:>7.1} ms",
                    started.elapsed().as_secs_f64() * 1000.0
                );
            }
            let new_task = task(project_id, t.list_id);
            let started = Instant::now();
            repo.set_task(&project_id, &new_task).await.unwrap();
            println!(
                "real add set_task {:>7.1} ms",
                started.elapsed().as_secs_f64() * 1000.0
            );
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let repo = TaskLocalAutomergeRepository::new(dir.path().to_path_buf())
            .await
            .unwrap();
        let project_id = ProjectId::new();
        let list_id = TaskListId::new();
        let mut tasks = Vec::new();

        for round in 0..12 {
            let mut t = task(project_id, list_id);
            if round % 2 == 1 {
                // 奇数回は既存タスクのステータス更新
                t = tasks.last().cloned().unwrap();
                t.status = TaskStatus::Completed;
            } else {
                tasks.push(t.clone());
            }
            let started = Instant::now();
            repo.set_task(&project_id, &t).await.unwrap();
            println!(
                "round {round:2} tasks={} set_task {:>7.1} ms",
                tasks.len(),
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
    });
}
