//! Browser build of the Flequit UI.
//!
//! **Stage 1: the UI only.** This crate renders the real `.slint` shell against
//! sample data. It is a layout and interaction preview — a way to look at the
//! app on a phone-sized viewport without a device, and to check the responsive
//! breakpoints — not a usable task manager.
//!
//! # Why it does not use `flequit-ui`
//!
//! `flequit-ui` depends on `flequit-core` and `flequit-infrastructure`, and so
//! on `sea-orm`/`sqlx`, none of which build for `wasm32-unknown-unknown`.
//! Nothing below the UI runs in a browser today:
//!
//! | Layer | Blocker |
//! |---|---|
//! | Storage | `sea-orm` + `sqlx-sqlite` have no wasm backend |
//! | Runtime | the multi-threaded Tokio runtime needs threads the browser does not give a wasm module by default |
//! | Platform | `directories`, `rfd`, `notify-rust`, `opener`, `trash`, `dark-light` are all desktop-only |
//!
//! None of that is going to be fixed by porting the storage layer to the
//! browser. The plan is that a browser build never stores anything locally: it
//! talks to a Flequit backend server, which is also what the desktop and mobile
//! builds will offer as an alternative to local storage. So there is no
//! IndexedDB or OPFS repository to write — what this crate is missing is an API
//! client, and the server it would talk to is not designed yet.
//!
//! See `plans/plan.md` sections 7 and 8.
//!
//! # Running it
//!
//! See `web/README.md`.

slint::include_modules!();

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use wasm_bindgen::prelude::wasm_bindgen;

/// Entry point. Called by the JavaScript glue that `wasm-bindgen` generates.
///
/// Not gated on `target_arch`: `cfg(target_arch = ...)` is reserved for
/// `flequit-platform`, and everything here builds and runs on the desktop too,
/// which is how the demo is checked without a wasm toolchain.
#[wasm_bindgen(start)]
pub fn main() {
    // Turns a Rust panic into a readable JS stack trace instead of the
    // "unreachable executed" the browser would otherwise show.
    console_error_panic_hook::set_once();

    let window = match AppWindow::new() {
        Ok(window) => window,
        Err(error) => {
            // No logging subscriber in this build; the console hook above is
            // the only sink, and it only sees panics.
            eprintln!("flequit-web: could not create the window: {error}");
            return;
        }
    };

    populate(&window);

    if let Err(error) = window.run() {
        eprintln!("flequit-web: the event loop failed: {error}");
    }
}

/// Fills the shell's globals with enough sample data to exercise the layout.
///
/// Deliberately small: the point is to see the sidebar, the list and the detail
/// pane at each breakpoint, not to mirror a real database.
fn populate(window: &AppWindow) {
    let state = window.global::<AppState>();

    let lists = vec![
        TaskListItem {
            id: "list-inbox".into(),
            project_id: "project-demo".into(),
            name: "Inbox".into(),
            task_count: 2,
            ..Default::default()
        },
        TaskListItem {
            id: "list-later".into(),
            project_id: "project-demo".into(),
            name: "Later".into(),
            task_count: 1,
            ..Default::default()
        },
    ];

    let projects = vec![ProjectItem {
        id: "project-demo".into(),
        name: "Demo".into(),
        short_label: "De".into(),
        task_lists: ModelRc::new(VecModel::from(lists)),
        expanded: true,
        ..Default::default()
    }];

    let tasks = vec![
        sample_task("task-1", "list-inbox", "Try the responsive layout", false),
        sample_task("task-2", "list-inbox", "Resize the window to 600px", false),
        sample_task("task-3", "list-later", "Read plans/plan.md", true),
    ];

    let selected = tasks[0].clone();
    state.set_projects(ModelRc::new(VecModel::from(projects)));
    state.set_tasks(ModelRc::new(VecModel::from(tasks)));
    state.set_selected_project_id(SharedString::from("project-demo"));
    state.set_add_target_list_id(SharedString::from("list-inbox"));
    state.set_add_target_project_name(SharedString::from("Demo"));
    state.set_add_target_list_name(SharedString::from("Inbox"));
    state.set_selected_task_id(selected.id.clone());
    state.set_selected_task(selected);
    state.set_has_selected_task(true);
}

fn sample_task(id: &str, list_id: &str, title: &str, completed: bool) -> TaskItem {
    TaskItem {
        id: id.into(),
        project_id: "project-demo".into(),
        list_id: list_id.into(),
        title: title.into(),
        completed,
        status: if completed {
            TaskStatus::Completed
        } else {
            TaskStatus::NotStarted
        },
        priority: TaskPriority::Medium,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use slint::Model;

    use super::*;

    #[test]
    fn the_demo_publishes_a_selectable_task() {
        // Runs headless so the assertion does not need a display.
        i_slint_backend_testing::init_no_event_loop();

        let window = AppWindow::new().expect("window");
        populate(&window);

        let state = window.global::<AppState>();
        assert_eq!(state.get_projects().row_count(), 1);
        assert_eq!(state.get_tasks().row_count(), 3);
        assert!(state.get_has_selected_task());
        assert_eq!(state.get_selected_task().title, "Try the responsive layout");
    }
}
