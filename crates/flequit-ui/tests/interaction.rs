//! Verifies that user actions reach their handlers.
//!
//! The UI shell once rendered correctly while almost nothing responded to a
//! click: several callbacks had no Rust handler, and the ones that did produced
//! no visible change. These tests pin the wiring so that regression is caught
//! without a display.
//!
//! They drive the UI through the accessibility tree, which is also how a screen
//! reader and keyboard navigation reach these controls — so a control that is
//! not reachable here is not operable without a mouse either.

use std::cell::RefCell;
use std::rc::Rc;

use flequit_ui::bindings::{
    Actions, AppState, AppWindow, DueFilterItem, ProjectItem, TaskItem, TaskListItem, TaskPriority,
    TaskStatus,
};
use i_slint_backend_testing::ElementHandle;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

/// Installs the headless backend exactly once per test binary.
fn init_backend() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(i_slint_backend_testing::init_no_event_loop);
}

fn task_list_item(id: &str, name: &str) -> TaskListItem {
    TaskListItem {
        id: SharedString::from(id),
        project_id: SharedString::from("p1"),
        name: SharedString::from(name),
        task_count: 0,
    }
}

fn project_item(expanded: bool) -> ProjectItem {
    ProjectItem {
        id: SharedString::from("p1"),
        name: SharedString::from("My Tasks"),
        short_label: SharedString::from("My"),
        color: SharedString::default(),
        has_color: false,
        expanded,
        task_lists: ModelRc::new(VecModel::from(vec![task_list_item("l1", "Inbox")])),
    }
}

fn task_item(id: &str, title: &str) -> TaskItem {
    TaskItem {
        id: SharedString::from(id),
        list_id: SharedString::from("l1"),
        title: SharedString::from(title),
        status: TaskStatus::NotStarted,
        priority: TaskPriority::None,
        completed: false,
        due_label: SharedString::default(),
        has_due: false,
        overdue: false,
        notes: SharedString::default(),
        tag_labels: ModelRc::new(VecModel::<SharedString>::default()),
        subtasks: ModelRc::new(VecModel::default()),
        subtask_count: 0,
        subtask_done_count: 0,
        expanded: false,
    }
}

/// Builds a window with content in place and the loading veil down.
///
/// `show()` is required: repeated rows (`for` blocks) are only instantiated once
/// the item tree has been laid out, and until then the accessibility tree is
/// empty.
fn window_with_content() -> AppWindow {
    init_backend();
    let window = AppWindow::new().expect("failed to create the window");
    // Assertions compare against English labels, so pin the locale: otherwise the
    // result depends on the developer's system language.
    // Must run after the first component exists; that is a Slint requirement.
    slint::select_bundled_translation("en").expect("no bundled English translation");
    window.show().expect("failed to show the window");
    let state = window.global::<AppState>();
    state.set_loading(false);
    state.set_projects(ModelRc::new(VecModel::from(vec![project_item(true)])));
    state.set_tasks(ModelRc::new(VecModel::from(vec![task_item(
        "t1", "Buy milk",
    )])));
    state.set_due_filters(ModelRc::new(VecModel::from(vec![DueFilterItem {
        key: SharedString::from("today"),
        label: SharedString::from("today"),
        query: SharedString::from("@today"),
        count: 0,
        visible: true,
    }])));
    window
}

/// Lists every labelled control, for diagnosing a failed lookup.
fn accessible_labels(window: &AppWindow) -> Vec<String> {
    i_slint_backend_testing::ElementQuery::from_root(window)
        .match_descendants()
        .find_all()
        .into_iter()
        .filter_map(|e| e.accessible_label().map(|l| l.to_string()))
        .filter(|l| !l.is_empty())
        .collect()
}

/// Activates the first control carrying `label`; reports whether it was found.
///
/// Mirrors what a screen reader or keyboard activation does.
fn activate(window: &AppWindow, label: &str) -> bool {
    match ElementHandle::find_by_accessible_label(window, label).next() {
        Some(element) => {
            element.invoke_accessible_default_action();
            true
        }
        None => false,
    }
}

fn selecting_a_project_reaches_its_handler() {
    let window = window_with_content();
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let seen = Rc::clone(&seen);
        window
            .global::<Actions>()
            .on_select_project(move |id| seen.borrow_mut().push(id.to_string()));
    }

    assert!(
        activate(&window, "My Tasks"),
        "the project row is not reachable through the accessibility tree"
    );
    assert_eq!(seen.borrow().as_slice(), ["p1"]);
}

fn selecting_a_task_list_reaches_its_handler() {
    let window = window_with_content();
    let seen = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
    {
        let seen = Rc::clone(&seen);
        window
            .global::<Actions>()
            .on_select_task_list(move |project, list| {
                seen.borrow_mut()
                    .push((project.to_string(), list.to_string()))
            });
    }

    assert!(
        activate(&window, "Inbox"),
        "the task list row is not reachable"
    );
    assert_eq!(
        seen.borrow().as_slice(),
        [("p1".to_string(), "l1".to_string())]
    );
}

fn a_due_filter_reaches_its_handler() {
    let window = window_with_content();
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let seen = Rc::clone(&seen);
        window
            .global::<Actions>()
            .on_due_filter_clicked(move |key| seen.borrow_mut().push(key.to_string()));
    }

    assert!(
        activate(&window, "Today"),
        "the due filter is not reachable"
    );
    assert_eq!(seen.borrow().as_slice(), ["today"]);
}

fn a_task_row_can_be_selected_and_completed() {
    let window = window_with_content();

    let selected = Rc::new(RefCell::new(Vec::<String>::new()));
    let toggled = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let selected = Rc::clone(&selected);
        window
            .global::<Actions>()
            .on_select_task(move |id| selected.borrow_mut().push(id.to_string()));
        let toggled = Rc::clone(&toggled);
        window
            .global::<Actions>()
            .on_toggle_task_completed(move |id| toggled.borrow_mut().push(id.to_string()));
    }

    // Both the row (select) and its checkbox (complete) carry the task title,
    // so activate every match and assert each handler saw exactly one call.
    let mut found = 0;
    for element in ElementHandle::find_by_accessible_label(&window, "Buy milk") {
        element.invoke_accessible_default_action();
        found += 1;
    }
    assert!(
        found >= 2,
        "expected a selectable row and a checkbox, found {found}"
    );

    assert_eq!(selected.borrow().as_slice(), ["t1"]);
    assert_eq!(toggled.borrow().as_slice(), ["t1"]);
}

fn expanding_a_project_reaches_its_handler() {
    let window = window_with_content();
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let seen = Rc::clone(&seen);
        window
            .global::<Actions>()
            .on_toggle_project_expansion(move |id| seen.borrow_mut().push(id.to_string()));
    }

    assert!(
        activate(&window, "Expand My Tasks"),
        "the project chevron is not reachable; expansion would need a double click"
    );
    assert_eq!(seen.borrow().as_slice(), ["p1"]);
}

fn the_settings_button_is_reachable() {
    let window = window_with_content();
    let pressed = Rc::new(RefCell::new(0usize));
    {
        let pressed = Rc::clone(&pressed);
        window
            .global::<Actions>()
            .on_open_settings(move || *pressed.borrow_mut() += 1);
    }

    assert!(
        activate(&window, "Settings"),
        "the settings button is not reachable; labelled controls: {:?}",
        accessible_labels(&window)
    );
    assert_eq!(*pressed.borrow(), 1);
}

/// Slint's backend is process-global and its components are `!Send`, so every
/// case runs inside one test on one thread.
#[test]
fn the_shell_responds_to_user_actions() {
    selecting_a_project_reaches_its_handler();
    selecting_a_task_list_reaches_its_handler();
    a_due_filter_reaches_its_handler();
    a_task_row_can_be_selected_and_completed();
    expanding_a_project_reaches_its_handler();
    the_settings_button_is_reachable();
}
