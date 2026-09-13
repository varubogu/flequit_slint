use flequit_model::models::task_projects::task::{PartialTask, TaskTree};

pub(super) fn apply_task_patch(task: &mut TaskTree, patch: &PartialTask) {
    if let Some(value) = patch.list_id {
        task.list_id = value;
    }
    if let Some(value) = patch.title.as_ref() {
        task.title.clone_from(value);
    }
    if let Some(value) = patch.description.as_ref() {
        task.description.clone_from(value);
    }
    if let Some(value) = patch.status.as_ref() {
        task.status = value.clone();
    }
    if let Some(value) = patch.priority {
        task.priority = value;
    }
    if let Some(value) = patch.plan_start_date {
        task.plan_start_date = value;
    }
    if let Some(value) = patch.plan_end_date {
        task.plan_end_date = value;
    }
    if let Some(value) = patch.do_start_date {
        task.do_start_date = value;
    }
    if let Some(value) = patch.do_end_date {
        task.do_end_date = value;
    }
    if let Some(value) = patch.is_range_date {
        task.is_range_date = value;
    }
    if let Some(value) = patch.recurrence_rule.as_ref() {
        task.recurrence_rule.clone_from(value);
    }
    if let Some(value) = patch.reminders.as_ref() {
        task.reminders.clone_from(value);
    }
    if let Some(value) = patch.order_index {
        task.order_index = value;
    }
    if let Some(value) = patch.is_archived {
        task.is_archived = value;
    }
    if let Some(value) = patch.assigned_user_ids.as_ref() {
        task.assigned_user_ids.clone_from(value);
    }
    if let Some(value) = patch.tag_ids.as_ref() {
        task.tag_ids.clone_from(value);
    }
    if let Some(value) = patch.created_at {
        task.created_at = value;
    }
    if let Some(value) = patch.updated_at {
        task.updated_at = value;
    }
    if let Some(value) = patch.deleted {
        task.deleted = value;
    }
    if let Some(value) = patch.updated_by {
        task.updated_by = value;
    }
}
