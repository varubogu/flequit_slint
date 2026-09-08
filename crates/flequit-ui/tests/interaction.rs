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

use flequit_ui::bindings::DueUnit;
use flequit_ui::bindings::{
    Actions, AppState, AppWindow, BookmarkedTagItem, Capabilities, ColorOption, DueButtonSetting,
    DueFilterItem, EditorKind, Layout, ProjectItem, RecurrenceEnd, RecurrenceMonthlyMode,
    RecurrencePresetSetting, RecurrenceState, RecurrenceUnit, RecurrenceWeekOfMonth, ReminderItem,
    SearchSuggestion, SearchSuggestionKind, SettingsCategory, SettingsState, SubTaskItem, TagItem,
    TaskItem, TaskListItem, TaskPriority, TaskSort, TaskStatus, Theme, ThemeMode,
};
use i_slint_backend_testing::ElementHandle;
use slint::{Brush, Color, ComponentHandle, Model, ModelRc, SharedString, VecModel};

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
        color_brush: Brush::default(),
        has_color: false,
        is_archived: false,
        expanded,
        task_lists: ModelRc::new(VecModel::from(vec![task_list_item("l1", "Inbox")])),
    }
}

fn task_item(id: &str, title: &str) -> TaskItem {
    TaskItem {
        id: SharedString::from(id),
        project_id: SharedString::from("p1"),
        list_id: SharedString::from("l1"),
        title: SharedString::from(title),
        status: TaskStatus::NotStarted,
        priority: TaskPriority::None,
        completed: false,
        due_label: SharedString::default(),
        has_due: false,
        overdue: false,
        due_year: 2026,
        due_month: 9,
        due_day: 6,
        due_hour: 12,
        due_minute: 0,
        is_range_date: false,
        start_label: SharedString::default(),
        has_start: false,
        start_year: 2026,
        start_month: 9,
        start_day: 6,
        start_hour: 12,
        start_minute: 0,
        notes: SharedString::default(),
        tag_labels: ModelRc::new(VecModel::<SharedString>::default()),
        subtasks: ModelRc::new(VecModel::default()),
        subtask_count: 0,
        subtask_done_count: 0,
        has_recurrence: false,
        recurrence_unit: RecurrenceUnit::Day,
        recurrence_interval: 1,
        reminders: ModelRc::new(VecModel::from(vec![ReminderItem {
            key: SharedString::from("2026-09-07T12:00:00+00:00"),
            label: SharedString::from("2026-09-07 12:00"),
        }])),
        expanded: false,
    }
}

fn subtask_item(id: &str, title: &str) -> SubTaskItem {
    SubTaskItem {
        id: SharedString::from(id),
        task_id: SharedString::from("t1"),
        title: SharedString::from(title),
        status: TaskStatus::NotStarted,
        priority: TaskPriority::None,
        completed: false,
        notes: SharedString::default(),
        due_label: SharedString::default(),
        has_due: false,
        overdue: false,
        due_year: 2026,
        due_month: 9,
        due_day: 6,
        due_hour: 12,
        due_minute: 0,
    }
}

/// A weekly draft, as the ViewModel would publish it when the editor opens.
fn recurrence_state(task_id: &str) -> RecurrenceState {
    RecurrenceState {
        task_id: SharedString::from(task_id),
        enabled: true,
        unit: RecurrenceUnit::Week,
        interval: 1,
        sunday: false,
        monday: false,
        tuesday: false,
        wednesday: false,
        thursday: false,
        friday: false,
        saturday: false,
        monthly_mode: RecurrenceMonthlyMode::SameDay,
        day_of_month: 6,
        week_of_month: RecurrenceWeekOfMonth::First,
        weekday_of_month: 0,
        end_kind: RecurrenceEnd::Never,
        end_year: 2026,
        end_month: 9,
        end_day: 6,
        max_occurrences: 10,
        preview_count: 5,
    }
}

fn tag_item(id: &str, name: &str, bookmarked: bool, assigned: bool) -> TagItem {
    TagItem {
        id: SharedString::from(id),
        project_id: SharedString::from("p1"),
        name: SharedString::from(name),
        color: SharedString::from("#4c6ef5"),
        color_brush: Color::from_rgb_u8(0x4c, 0x6e, 0xf5).into(),
        has_color: true,
        bookmarked,
        assigned,
    }
}

fn bookmarked_tag_item(id: &str, name: &str) -> BookmarkedTagItem {
    BookmarkedTagItem {
        id: SharedString::from(id),
        project_id: SharedString::from("p1"),
        name: SharedString::from(name),
        color_brush: Color::from_rgb_u8(0x4c, 0x6e, 0xf5).into(),
        has_color: true,
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
        custom_value: 0,
        custom_unit: DueUnit::Day,
    }])));
    window
        .global::<SettingsState>()
        .set_due_buttons(ModelRc::new(VecModel::from(vec![DueButtonSetting {
            key: SharedString::from("today"),
            visible: true,
        }])));
    let settings = window.global::<SettingsState>();
    settings.set_timezone("UTC".into());
    settings.set_datetime_format_names(ModelRc::new(VecModel::from(vec![
        "Default".into(),
        "Japan (24-hour)".into(),
        "United States (24-hour)".into(),
        "ISO 8601".into(),
        "Custom".into(),
    ])));
    settings.set_datetime_format_ids(ModelRc::new(VecModel::from(vec![
        "-1".into(),
        "jp-0".into(),
        "en-0".into(),
        "iso-0".into(),
        "-2".into(),
    ])));
    settings.set_datetime_format_values(ModelRc::new(VecModel::from(vec![
        "".into(),
        "%Y年%m月%d日 %H:%M:%S".into(),
        "%m/%d/%Y %H:%M:%S".into(),
        "%Y-%m-%dT%H:%M:%S%:z".into(),
        "".into(),
    ])));
    settings.set_datetime_format_kinds(ModelRc::new(VecModel::from(vec![
        "default".into(),
        "preset".into(),
        "preset".into(),
        "preset".into(),
        "custom".into(),
    ])));
    state.set_project_colors(ModelRc::new(VecModel::from(vec![ColorOption {
        value: SharedString::from("#4c6ef5"),
        swatch: Color::from_rgb_u8(0x4c, 0x6e, 0xf5).into(),
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

fn set_value(window: &AppWindow, label: &str, value: &str) {
    let input = ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_value().is_some())
        .unwrap_or_else(|| panic!("input {label:?} is not reachable"));
    input.set_accessible_value(value);
    settle();
    assert_eq!(input.accessible_value().as_deref(), Some(value));
}

/// Runs pending `changed` handlers.
///
/// Without an event loop nothing else does, so a property written from a test
/// would never reach the handlers a real interaction would trigger.
fn settle() {
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::ZERO);
}

/// Sends one key press and release, the way a keyboard user reaches a control.
///
/// Tab traversal and Space activation are handled by Slint itself, so they can
/// only be exercised through real key events, not through the accessibility
/// actions the other helpers use.
fn press_key(window: &AppWindow, key: char) {
    let text = SharedString::from(key.to_string());
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    settle();
}

/// Steps a control that supports incremental adjustment, such as a drag handle.
///
/// This is the path a keyboard or screen reader takes when a pointer drag is
/// not available, so it is also the only way to exercise reordering headlessly.
fn adjust(window: &AppWindow, label: &str, forward: bool) -> bool {
    match ElementHandle::find_by_accessible_label(window, label).next() {
        Some(element) => {
            if forward {
                element.invoke_accessible_increment_action();
            } else {
                element.invoke_accessible_decrement_action();
            }
            true
        }
        None => false,
    }
}

/// Resizes the real window and lets the shell publish the new width.
///
/// `Layout.window-width` follows `Window.width`, so setting the size is what a
/// resize does; writing the global directly would leave the actual geometry at
/// the default and put hit testing at coordinates the layout never used.
fn resize(window: &AppWindow, width: f32, height: f32) {
    window
        .window()
        .set_size(slint::LogicalSize::new(width, height));
    settle();
}

/// Clicks the first control carrying `label` with a pointer press and release
/// at the element's centre; reports whether such a control exists.
///
/// Unlike [`activate`], this goes through Slint's hit testing: the events land
/// on whichever element is topmost at that point. A control that is covered by
/// a modal scrim, or that has collapsed to zero size, therefore does not
/// respond — which is the property these tests are for.
fn click(window: &AppWindow, label: &str) -> bool {
    match ElementHandle::find_by_accessible_label(window, label).next() {
        Some(element) => {
            element.mock_single_click(slint::platform::PointerEventButton::Left);
            settle();
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

fn a_search_suggestion_reaches_its_handler() {
    let window = window_with_content();
    window
        .global::<AppState>()
        .set_search_suggestions(ModelRc::new(VecModel::from(vec![SearchSuggestion {
            query: SharedString::from("@today"),
            replacement: SharedString::from("@today"),
            kind: SearchSuggestionKind::Due,
        }])));

    let searches = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let searches = Rc::clone(&searches);
        window
            .global::<Actions>()
            .on_search_changed(move |query| searches.borrow_mut().push(query.to_string()));
    }

    set_value(&window, "Search tasks", "@to");
    assert!(
        activate(&window, "Use search suggestion @today"),
        "the search suggestion is not reachable: {:?}",
        accessible_labels(&window)
    );
    assert_eq!(searches.borrow().as_slice(), ["@to", "@today"]);
}

fn project_and_task_list_management_reaches_its_handlers() {
    let window = window_with_content();
    let created_projects = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
    let updated_projects = Rc::new(RefCell::new(Vec::<(String, String, String)>::new()));
    let archived_projects = Rc::new(RefCell::new(Vec::<(String, bool)>::new()));
    let deleted_projects = Rc::new(RefCell::new(Vec::<String>::new()));
    let created_lists = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
    let updated_lists = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
    let deleted_lists = Rc::new(RefCell::new(Vec::<String>::new()));
    let archived_toggles = Rc::new(RefCell::new(0usize));

    {
        let seen = Rc::clone(&created_projects);
        window
            .global::<Actions>()
            .on_create_project(move |name, color| {
                seen.borrow_mut()
                    .push((name.to_string(), color.to_string()));
            });
        let seen = Rc::clone(&updated_projects);
        window
            .global::<Actions>()
            .on_update_project(move |id, name, color| {
                seen.borrow_mut()
                    .push((id.to_string(), name.to_string(), color.to_string()));
            });
        let seen = Rc::clone(&archived_projects);
        window
            .global::<Actions>()
            .on_archive_project(move |id, archived| {
                seen.borrow_mut().push((id.to_string(), archived));
            });
        let seen = Rc::clone(&deleted_projects);
        window
            .global::<Actions>()
            .on_delete_project(move |id| seen.borrow_mut().push(id.to_string()));
        let seen = Rc::clone(&created_lists);
        window
            .global::<Actions>()
            .on_create_task_list(move |project_id, name| {
                seen.borrow_mut()
                    .push((project_id.to_string(), name.to_string()));
            });
        let seen = Rc::clone(&updated_lists);
        window
            .global::<Actions>()
            .on_update_task_list(move |id, name| {
                seen.borrow_mut().push((id.to_string(), name.to_string()));
            });
        let seen = Rc::clone(&deleted_lists);
        window
            .global::<Actions>()
            .on_delete_task_list(move |id| seen.borrow_mut().push(id.to_string()));
        let seen = Rc::clone(&archived_toggles);
        window
            .global::<Actions>()
            .on_toggle_archived_projects(move || *seen.borrow_mut() += 1);
    }

    assert!(activate(&window, "New project"));
    settle();
    set_value(&window, "Name", "Work");
    assert!(activate(&window, "Colour #4c6ef5"));
    assert!(activate(&window, "Save"));
    assert_eq!(
        created_projects.borrow().as_slice(),
        [("Work".to_string(), "#4c6ef5".to_string())]
    );

    assert!(activate(&window, "Edit My Tasks"));
    settle();
    set_value(&window, "Name", "Personal");
    assert!(activate(&window, "Save"));
    assert_eq!(
        updated_projects.borrow().as_slice(),
        [("p1".to_string(), "Personal".to_string(), String::new())]
    );

    assert!(activate(&window, "Edit My Tasks"));
    settle();
    assert!(activate(&window, "Archive"));
    assert_eq!(
        archived_projects.borrow().as_slice(),
        [("p1".to_string(), true)]
    );

    assert!(activate(&window, "Add a list to My Tasks"));
    settle();
    set_value(&window, "Name", "Next");
    assert!(activate(&window, "Save"));
    assert_eq!(
        created_lists.borrow().as_slice(),
        [("p1".to_string(), "Next".to_string())]
    );

    assert!(activate(&window, "Edit Inbox"));
    settle();
    set_value(&window, "Name", "Later");
    assert!(activate(&window, "Save"));
    assert_eq!(
        updated_lists.borrow().as_slice(),
        [("l1".to_string(), "Later".to_string())]
    );

    assert!(activate(&window, "Edit Inbox"));
    settle();
    assert!(activate(&window, "Delete"));
    settle();
    assert!(activate(&window, "Delete for good"));
    assert_eq!(deleted_lists.borrow().as_slice(), ["l1"]);

    assert!(activate(&window, "Edit My Tasks"));
    settle();
    assert!(activate(&window, "Delete"));
    settle();
    assert!(activate(&window, "Delete for good"));
    assert_eq!(deleted_projects.borrow().as_slice(), ["p1"]);

    assert!(activate(&window, "Show archived projects"));
    assert_eq!(*archived_toggles.borrow(), 1);
}

fn the_settings_dialog_reaches_its_handlers() {
    let window = window_with_content();
    let week_starts = Rc::new(RefCell::new(Vec::<String>::new()));
    let vim_modes = Rc::new(RefCell::new(Vec::<bool>::new()));
    let due_buttons = Rc::new(RefCell::new(Vec::<(String, bool)>::new()));
    let custom_due_filters = Rc::new(RefCell::new(Vec::<(i32, DueUnit)>::new()));
    let searches = Rc::new(RefCell::new(Vec::<String>::new()));
    let themes = Rc::new(RefCell::new(Vec::<ThemeMode>::new()));
    let timezones = Rc::new(RefCell::new(Vec::<String>::new()));
    let timezone_queries = Rc::new(RefCell::new(Vec::<String>::new()));
    let datetime_formats = Rc::new(RefCell::new(Vec::<String>::new()));
    let custom_formats = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
    {
        let weak = window.as_weak();
        window.global::<Actions>().on_open_settings(move || {
            if let Some(window) = weak.upgrade() {
                let settings = window.global::<SettingsState>();
                settings.set_open(true);
                settings.set_selected_category(flequit_ui::bindings::SettingsCategory::Basic);
                settings.set_compact_detail_open(false);
            }
        });
        let weak = window.as_weak();
        window.global::<Actions>().on_close_settings(move || {
            if let Some(window) = weak.upgrade() {
                window.global::<SettingsState>().set_open(false);
            }
        });
        let weak = window.as_weak();
        window
            .global::<Actions>()
            .on_select_settings_category(move |category| {
                if let Some(window) = weak.upgrade() {
                    let settings = window.global::<SettingsState>();
                    settings.set_selected_category(category);
                    settings.set_compact_detail_open(true);
                }
            });
        let seen = Rc::clone(&week_starts);
        window
            .global::<Actions>()
            .on_update_week_start(move |value| seen.borrow_mut().push(value.to_string()));
        let seen = Rc::clone(&vim_modes);
        window
            .global::<Actions>()
            .on_update_vim_mode(move |enabled| seen.borrow_mut().push(enabled));
        let seen = Rc::clone(&searches);
        window
            .global::<Actions>()
            .on_search_settings(move |query, _, _, _, _| {
                seen.borrow_mut().push(query.to_string());
            });
        let seen = Rc::clone(&due_buttons);
        window
            .global::<Actions>()
            .on_update_due_button(move |key, visible| {
                seen.borrow_mut().push((key.to_string(), visible));
            });
        let seen = Rc::clone(&custom_due_filters);
        window
            .global::<Actions>()
            .on_add_custom_due_filter(move |value, unit| seen.borrow_mut().push((value, unit)));
        let seen = Rc::clone(&themes);
        window
            .global::<Actions>()
            .on_update_theme_mode(move |mode| seen.borrow_mut().push(mode));
        let seen = Rc::clone(&timezones);
        window
            .global::<Actions>()
            .on_update_timezone(move |value| seen.borrow_mut().push(value.to_string()));
        let seen = Rc::clone(&timezone_queries);
        let weak = window.as_weak();
        window
            .global::<Actions>()
            .on_filter_timezones(move |query| {
                seen.borrow_mut().push(query.to_string());
                // Stands in for the ViewModel so the dropdown has something to pick.
                if let Some(window) = weak.upgrade() {
                    window
                        .global::<SettingsState>()
                        .set_timezone_options(ModelRc::new(VecModel::from(vec![
                            SharedString::from("Asia/Tokyo"),
                        ])));
                }
            });
        let seen = Rc::clone(&datetime_formats);
        window
            .global::<Actions>()
            .on_update_datetime_format(move |value| seen.borrow_mut().push(value.to_string()));
        let seen = Rc::clone(&custom_formats);
        window
            .global::<Actions>()
            .on_add_datetime_format(move |name, value| {
                seen.borrow_mut()
                    .push((name.to_string(), value.to_string()));
            });
        window
            .global::<Actions>()
            .on_datetime_format_index(|value| if value == "%d/%m/%Y" { 4 } else { 0 });
        window
            .global::<Actions>()
            .on_preview_datetime(|_, _, _, _, _, _| "2026-09-06 12:00".into());
    }

    assert!(activate(&window, "Open account menu"));
    settle();
    assert!(
        activate(&window, "Settings"),
        "the settings button is not reachable; labelled controls: {:?}",
        accessible_labels(&window)
    );
    settle();
    assert!(window.global::<SettingsState>().get_open());
    set_value(&window, "Search settings", "font");
    assert_eq!(searches.borrow().as_slice(), ["font"]);
    assert!(activate(&window, "Monday"));
    assert_eq!(week_starts.borrow().as_slice(), ["monday"]);
    assert!(activate(
        &window,
        "Enable Vim task navigation (j/k and g/G)"
    ));
    assert_eq!(vim_modes.borrow().as_slice(), [true]);
    assert!(activate(&window, "Show Today due filter"));
    assert_eq!(
        due_buttons.borrow().as_slice(),
        [("today".to_string(), false)]
    );
    assert!(activate(&window, "Add due filter"));
    assert_eq!(custom_due_filters.borrow().as_slice(), [(5, DueUnit::Day)]);
    // A horizon shorter than a day is added the same way, with a unit picked first.
    assert!(activate(&window, "Minutes"));
    assert!(activate(&window, "Add due filter"));
    assert_eq!(
        custom_due_filters.borrow().last(),
        Some(&(5, DueUnit::Minute))
    );

    assert!(activate(&window, "Date and time"));
    settle();
    // The timezone field narrows the list as you type; only picking an entry
    // out of the dropdown commits it, so a half-typed name is never stored.
    set_value(&window, "Timezone", "Asia/T");
    settle();
    assert_eq!(timezone_queries.borrow().as_slice(), ["Asia/T"]);
    assert!(timezones.borrow().is_empty());
    assert!(
        activate(&window, "Asia/Tokyo"),
        "the filtered timezone is not reachable; labelled controls: {:?}",
        accessible_labels(&window)
    );
    assert_eq!(timezones.borrow().as_slice(), ["Asia/Tokyo"]);
    set_value(&window, "Current date and time format", "%Y/%m/%d");
    assert_eq!(datetime_formats.borrow().as_slice(), ["%Y/%m/%d"]);
    set_value(&window, "Test date and time format", "%d/%m/%Y");
    set_value(&window, "Custom format name", "European");
    assert!(activate(&window, "Add custom format"));
    assert_eq!(
        custom_formats.borrow().as_slice(),
        [("European".to_string(), "%d/%m/%Y".to_string())]
    );

    assert!(activate(&window, "Appearance"));
    settle();
    assert!(activate(&window, "Use Dark theme"));
    assert_eq!(themes.borrow().as_slice(), [ThemeMode::Dark]);
    assert!(activate(&window, "Close settings"));
    assert!(!window.global::<SettingsState>().get_open());

    window.global::<Layout>().set_window_width(480.0);
    assert!(activate(&window, "Open menu"));
    assert!(activate(&window, "Open account menu"));
    settle();
    assert!(activate(&window, "Settings"));
    settle();
    assert!(activate(&window, "Account"));
    settle();
    assert!(
        ElementHandle::find_by_accessible_label(&window, "Back to settings categories")
            .next()
            .is_some(),
        "compact settings did not enter the detail level"
    );
    assert!(activate(&window, "Back to settings categories"));
    assert!(!window.global::<SettingsState>().get_compact_detail_open());
}

fn the_account_menu_reaches_its_destinations() {
    let window = window_with_content();
    let settings_opened = Rc::new(RefCell::new(0));
    let categories = Rc::new(RefCell::new(Vec::<SettingsCategory>::new()));
    let help_opened = Rc::new(RefCell::new(0));
    window
        .global::<SettingsState>()
        .set_account_name("Alice".into());

    {
        let count = Rc::clone(&settings_opened);
        window
            .global::<Actions>()
            .on_open_settings(move || *count.borrow_mut() += 1);
        let seen = Rc::clone(&categories);
        window
            .global::<Actions>()
            .on_select_settings_category(move |category| seen.borrow_mut().push(category));
        let count = Rc::clone(&help_opened);
        window
            .global::<Actions>()
            .on_open_help(move || *count.borrow_mut() += 1);
    }

    assert!(activate(&window, "Open account menu"));
    settle();
    assert!(activate(&window, "Settings"));
    assert_eq!(*settings_opened.borrow(), 1);

    assert!(activate(&window, "Open account menu"));
    settle();
    assert!(activate(&window, "Account settings"));
    assert_eq!(*settings_opened.borrow(), 2);
    assert_eq!(categories.borrow().as_slice(), [SettingsCategory::Account]);

    assert!(activate(&window, "Open account menu"));
    settle();
    assert!(activate(&window, "Help"));
    assert_eq!(*help_opened.borrow(), 1);
}

fn task_list_empty_states_offer_a_next_action() {
    let window = window_with_content();
    let state = window.global::<AppState>();
    state.set_tasks(ModelRc::new(VecModel::default()));

    let searches = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let seen = Rc::clone(&searches);
        let weak = window.as_weak();
        window.global::<Actions>().on_search_changed(move |query| {
            seen.borrow_mut().push(query.to_string());
            if let Some(window) = weak.upgrade() {
                window.global::<AppState>().set_search_query(query);
            }
        });
    }

    state.set_search_query("missing".into());
    settle();
    assert!(activate(&window, "Clear search"));
    assert_eq!(searches.borrow().as_slice(), [""]);

    state.set_projects(ModelRc::new(VecModel::default()));
    state.set_selected_project_id(SharedString::default());
    state.set_selected_list_id(SharedString::default());
    settle();
    assert!(activate(&window, "Create project"));
    assert!(state.get_editor_open());
    assert_eq!(state.get_editor().kind, EditorKind::Project);

    state.set_editor_open(false);
    state.set_projects(ModelRc::new(VecModel::from(vec![project_item(true)])));
    state.set_selected_project_id("p1".into());
    settle();
    assert!(activate(&window, "Create task list"));
    assert!(state.get_editor_open());
    assert_eq!(state.get_editor().kind, EditorKind::TaskList);
    assert_eq!(state.get_editor().project_id.as_str(), "p1");

    state.set_editor_open(false);
    state.set_selected_list_id("l1".into());
    settle();
    assert!(activate(&window, "Add your first task"));
    assert!(
        ElementHandle::find_by_accessible_label(&window, "Add a task")
            .next()
            .is_some()
    );
}

fn notification_permission_can_be_requested_from_settings() {
    let window = window_with_content();
    window.global::<Capabilities>().set_local_notification(true);
    window.global::<SettingsState>().set_open(true);
    settle();

    let requests = Rc::new(RefCell::new(0));
    {
        let requests = Rc::clone(&requests);
        window
            .global::<Actions>()
            .on_request_notification_permission(move || *requests.borrow_mut() += 1);
    }

    assert!(
        activate(&window, "Allow notifications"),
        "the notification permission button is not reachable: {:?}",
        accessible_labels(&window)
    );
    assert_eq!(*requests.borrow(), 1);
}

fn adding_a_subtask_reaches_its_handler() {
    let window = window_with_content();
    let state = window.global::<AppState>();
    let task = state
        .get_tasks()
        .row_data(0)
        .expect("the test task should exist");
    state.set_selected_task_id(task.id.clone());
    state.set_selected_task(task);
    state.set_has_selected_task(true);

    let seen = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
    {
        let seen = Rc::clone(&seen);
        window
            .global::<Actions>()
            .on_add_subtask(move |task_id, title| {
                seen.borrow_mut()
                    .push((task_id.to_string(), title.to_string()));
            });
    }

    let input = ElementHandle::find_by_accessible_label(&window, "New subtask title")
        .next()
        .expect("the subtask input is not reachable");
    input.set_accessible_value("Pick up bread");
    assert!(
        activate(&window, "Add a subtask"),
        "the add-subtask button is not reachable"
    );
    assert_eq!(
        seen.borrow().as_slice(),
        [("t1".to_string(), "Pick up bread".to_string())]
    );
}

fn the_due_date_editor_is_reachable() {
    let window = window_with_content();
    let state = window.global::<AppState>();
    let mut task = state
        .get_tasks()
        .row_data(0)
        .expect("the test task should exist");
    task.has_due = true;
    task.due_label = SharedString::from("2026-09-06 12:00");
    state.set_selected_task_id(task.id.clone());
    state.set_selected_task(task);
    state.set_has_selected_task(true);

    let cleared = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let cleared = Rc::clone(&cleared);
        window
            .global::<Actions>()
            .on_clear_task_due(move |task_id| cleared.borrow_mut().push(task_id.to_string()));
    }

    assert!(
        ElementHandle::find_by_accessible_label(&window, "Edit due date")
            .next()
            .is_some(),
        "the due-date picker is not reachable"
    );
    assert!(
        activate(&window, "Clear due date"),
        "the clear-due-date button is not reachable"
    );
    assert_eq!(cleared.borrow().as_slice(), ["t1"]);
}

fn reminder_controls_reach_their_handlers() {
    let window = window_with_content();
    window.global::<Capabilities>().set_local_notification(true);
    let state = window.global::<AppState>();
    let mut task = state
        .get_tasks()
        .row_data(0)
        .expect("the test task should exist");
    task.has_start = true;
    task.start_label = "2026-09-06 11:00".into();
    task.has_due = true;
    task.due_label = "2026-09-06 12:00".into();
    state.set_selected_task_id(task.id.clone());
    state.set_selected_task(task);
    state.set_has_selected_task(true);

    let removed = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
    {
        let removed = Rc::clone(&removed);
        window
            .global::<Actions>()
            .on_remove_reminder(move |task_id, key| {
                removed
                    .borrow_mut()
                    .push((task_id.to_string(), key.to_string()));
            });
    }

    let relative = Rc::new(RefCell::new(Vec::<(String, bool, i32)>::new()));
    {
        let relative = Rc::clone(&relative);
        window.global::<Actions>().on_add_relative_reminder(
            move |task_id, from_start, minutes_before| {
                relative
                    .borrow_mut()
                    .push((task_id.to_string(), from_start, minutes_before));
            },
        );
    }

    assert!(
        ElementHandle::find_by_accessible_label(&window, "Add a reminder")
            .next()
            .is_some(),
        "the add-reminder picker is not reachable"
    );
    assert!(activate(&window, "30 minutes before start"));
    assert!(activate(&window, "30 minutes before due"));
    assert_eq!(
        relative.borrow().as_slice(),
        [("t1".to_string(), true, 30), ("t1".to_string(), false, 30)]
    );
    assert!(activate(&window, "Remove reminder 2026-09-07 12:00"));
    assert_eq!(
        removed.borrow().as_slice(),
        [("t1".to_string(), "2026-09-07T12:00:00+00:00".to_string())]
    );
}

fn the_priority_editor_reaches_its_handler() {
    let window = window_with_content();
    let state = window.global::<AppState>();
    let task = state
        .get_tasks()
        .row_data(0)
        .expect("the test task should exist");
    state.set_selected_task_id(task.id.clone());
    state.set_selected_task(task);
    state.set_has_selected_task(true);

    // The options live in a `PopupWindow`, which is a window of its own and so
    // is not part of the item tree `ElementHandle` walks. What the test can
    // check is that the field is reachable, opens, and reports the value the
    // task carries.
    let field = ElementHandle::find_by_accessible_label(&window, "Change priority")
        .next()
        .expect("the priority field is not reachable");
    assert_eq!(field.accessible_value().as_deref(), Some("None"));
    field.invoke_accessible_default_action();
    settle();
}

fn the_status_editor_reaches_its_handler() {
    let window = window_with_content();
    let state = window.global::<AppState>();
    let task = state
        .get_tasks()
        .row_data(0)
        .expect("the test task should exist");
    state.set_selected_task_id(task.id.clone());
    state.set_selected_task(task);
    state.set_has_selected_task(true);

    // See `the_priority_editor_reaches_its_handler`: the options are in a
    // popup, so only the field itself can be driven from here.
    let field = ElementHandle::find_by_accessible_label(&window, "Change status")
        .next()
        .expect("the status field is not reachable");
    assert_eq!(field.accessible_value().as_deref(), Some("Not started"));
    field.invoke_accessible_default_action();
    settle();
}

fn the_expanded_sidebar_can_be_collapsed_and_reopened() {
    let window = window_with_content();
    let state = window.global::<AppState>();

    assert!(!state.get_sidebar_collapsed());
    assert!(
        activate(&window, "Toggle sidebar"),
        "the expanded sidebar toggle is not reachable"
    );
    assert!(state.get_sidebar_collapsed());

    assert!(
        activate(&window, "Toggle sidebar"),
        "the collapsed sidebar toggle is not reachable"
    );
    assert!(!state.get_sidebar_collapsed());
}

fn the_medium_sidebar_stays_narrow_without_a_toggle() {
    let window = window_with_content();
    window.global::<Layout>().set_window_width(800.0);

    assert!(
        ElementHandle::find_by_accessible_label(&window, "Toggle sidebar")
            .next()
            .is_none(),
        "the fixed-width medium sidebar should not expose an ineffective toggle"
    );
}

fn the_compact_sidebar_opens_and_closes_as_an_overlay() {
    let window = window_with_content();
    window.global::<Layout>().set_window_width(480.0);
    let state = window.global::<AppState>();

    assert!(!state.get_sidebar_open());
    assert!(
        activate(&window, "Open menu"),
        "the compact menu button is not reachable"
    );
    assert!(state.get_sidebar_open());

    assert!(
        activate(&window, "Toggle sidebar"),
        "the compact overlay close button is not reachable"
    );
    assert!(!state.get_sidebar_open());
}

fn a_task_row_shows_its_tags() {
    let window = window_with_content();
    let mut task = task_item("t1", "Buy milk");
    task.tag_labels = ModelRc::new(VecModel::from(vec![
        SharedString::from("shopping"),
        SharedString::from("home"),
    ]));
    window
        .global::<AppState>()
        .set_tasks(ModelRc::new(VecModel::from(vec![task])));

    let labels = accessible_labels(&window);
    // The "#" prefix matches how the tag is typed into the search box.
    for tag in ["#shopping", "#home"] {
        assert!(
            labels.iter().any(|label| label == tag),
            "the task row does not show {tag}; visible labels: {labels:?}"
        );
    }
}

fn tag_management_and_assignment_reach_their_handlers() {
    let window = window_with_content();
    let state = window.global::<AppState>();
    state.set_selected_project_id(SharedString::from("p1"));
    state.set_tags(ModelRc::new(VecModel::from(vec![tag_item(
        "tag-home", "home", false, false,
    )])));
    state.set_bookmarked_tags(ModelRc::new(VecModel::from(vec![bookmarked_tag_item(
        "tag-home", "home",
    )])));
    settle();

    let selected_bookmarks = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
    let created = Rc::new(RefCell::new(Vec::<(String, String, String)>::new()));
    let updated = Rc::new(RefCell::new(Vec::<(String, String, String, String)>::new()));
    let deleted = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
    let bookmarked = Rc::new(RefCell::new(Vec::<(String, String, bool)>::new()));
    {
        let seen = Rc::clone(&selected_bookmarks);
        window
            .global::<Actions>()
            .on_select_tag_bookmark(move |project_id, name| {
                seen.borrow_mut()
                    .push((project_id.to_string(), name.to_string()));
            });
        let seen = Rc::clone(&created);
        window
            .global::<Actions>()
            .on_create_tag(move |project_id, name, color| {
                seen.borrow_mut().push((
                    project_id.to_string(),
                    name.to_string(),
                    color.to_string(),
                ));
            });
        let seen = Rc::clone(&updated);
        window
            .global::<Actions>()
            .on_update_tag(move |project_id, tag_id, name, color| {
                seen.borrow_mut().push((
                    project_id.to_string(),
                    tag_id.to_string(),
                    name.to_string(),
                    color.to_string(),
                ));
            });
        let seen = Rc::clone(&deleted);
        window
            .global::<Actions>()
            .on_delete_tag(move |project_id, tag_id| {
                seen.borrow_mut()
                    .push((project_id.to_string(), tag_id.to_string()));
            });
        let seen = Rc::clone(&bookmarked);
        window
            .global::<Actions>()
            .on_set_tag_bookmarked(move |project_id, tag_id, value| {
                seen.borrow_mut()
                    .push((project_id.to_string(), tag_id.to_string(), value));
            });
    }

    assert!(activate(&window, "Filter by tag home"));
    assert_eq!(
        selected_bookmarks.borrow().as_slice(),
        [("p1".to_string(), "home".to_string())]
    );

    assert!(activate(&window, "Manage tags"));
    settle();
    assert!(activate(&window, "New tag"));
    settle();
    set_value(&window, "Tag name", "work");
    assert!(activate(&window, "Colour #4c6ef5"));
    assert!(activate(&window, "Save"));
    assert_eq!(
        created.borrow().as_slice(),
        [("p1".to_string(), "work".to_string(), "#4c6ef5".to_string())]
    );

    assert!(activate(&window, "Pin tag home"));
    assert_eq!(
        bookmarked.borrow().as_slice(),
        [("p1".to_string(), "tag-home".to_string(), true)]
    );

    assert!(activate(&window, "Edit tag home"));
    settle();
    set_value(&window, "Tag name", "house");
    assert!(activate(&window, "Save"));
    assert_eq!(
        updated.borrow().as_slice(),
        [(
            "p1".to_string(),
            "tag-home".to_string(),
            "house".to_string(),
            "#4c6ef5".to_string(),
        )]
    );

    assert!(activate(&window, "Edit tag home"));
    settle();
    assert!(activate(&window, "Delete"));
    assert!(activate(&window, "Delete for good"));
    assert_eq!(
        deleted.borrow().as_slice(),
        [("p1".to_string(), "tag-home".to_string())]
    );
    assert!(activate(&window, "Close"));

    let assigned = Rc::new(RefCell::new(Vec::<(String, String, bool)>::new()));
    let added = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
    {
        let seen = Rc::clone(&assigned);
        window
            .global::<Actions>()
            .on_set_task_tag(move |task_id, tag_id, value| {
                seen.borrow_mut()
                    .push((task_id.to_string(), tag_id.to_string(), value));
            });
        let seen = Rc::clone(&added);
        window
            .global::<Actions>()
            .on_add_task_tag(move |task_id, name| {
                seen.borrow_mut()
                    .push((task_id.to_string(), name.to_string()));
            });
    }
    state.set_tags(ModelRc::new(VecModel::from(vec![
        tag_item("tag-home", "home", false, true),
        tag_item("tag-work", "work", false, false),
    ])));
    state.set_selected_task(task_item("t1", "Buy milk"));
    state.set_selected_task_id(SharedString::from("t1"));
    state.set_has_selected_task(true);
    settle();

    assert!(activate(&window, "Remove tag home from Buy milk"));
    assert!(activate(&window, "Add tag work to Buy milk"));
    assert_eq!(
        assigned.borrow().as_slice(),
        [
            ("t1".to_string(), "tag-home".to_string(), false),
            ("t1".to_string(), "tag-work".to_string(), true),
        ]
    );

    set_value(&window, "New tag name", "errands");
    assert!(activate(&window, "Add tag"));
    assert_eq!(
        added.borrow().as_slice(),
        [("t1".to_string(), "errands".to_string())]
    );
}

fn the_sort_bar_reaches_its_handler() {
    let window = window_with_content();
    let seen = Rc::new(RefCell::new(Vec::<TaskSort>::new()));
    {
        let seen = Rc::clone(&seen);
        window
            .global::<Actions>()
            .on_change_task_sort(move |sort| seen.borrow_mut().push(sort));
    }

    for (label, expected) in [
        ("Sort by Due date", TaskSort::Due),
        ("Sort by Priority", TaskSort::Priority),
        ("Sort by Name", TaskSort::Title),
        ("Sort by Manual", TaskSort::Manual),
    ] {
        assert!(
            activate(&window, label),
            "the sort button {label:?} is not reachable; visible labels: {:?}",
            accessible_labels(&window)
        );
        assert_eq!(seen.borrow().last(), Some(&expected));
    }
}

/// Reordering has to be operable without a pointer, so the drag handle also
/// answers to the increment and decrement actions.
fn the_drag_handle_reaches_its_handler() {
    let window = window_with_content();
    window
        .global::<AppState>()
        .set_tasks(ModelRc::new(VecModel::from(vec![
            task_item("t1", "Buy milk"),
            task_item("t2", "Pick up bread"),
        ])));

    let seen = Rc::new(RefCell::new(Vec::<(String, i32)>::new()));
    {
        let seen = Rc::clone(&seen);
        window
            .global::<Actions>()
            .on_reorder_task(move |id, index| seen.borrow_mut().push((id.to_string(), index)));
    }

    assert!(
        adjust(&window, "Reorder Buy milk", true),
        "the drag handle is not reachable; visible labels: {:?}",
        accessible_labels(&window)
    );
    assert!(adjust(&window, "Reorder Pick up bread", false));

    assert_eq!(
        seen.borrow().as_slice(),
        [("t1".to_string(), 1), ("t2".to_string(), 0)]
    );
}

/// A task list in the sidebar accepts a task dragged onto it.
///
/// The drop target has to recognise itself from the pointer position, because
/// the row being dragged keeps the pointer grab for the whole gesture.
fn a_sidebar_list_becomes_a_drop_target() {
    let window = window_with_content();
    let state = window.global::<AppState>();

    let list_row = ElementHandle::find_by_accessible_label(&window, "Inbox")
        .next()
        .expect("the task list row is not reachable");
    let position = list_row.absolute_position();
    let size = list_row.size();

    state.set_dragging_task_id(SharedString::from("t1"));
    state.set_dragging_project_id(SharedString::from("p1"));
    state.set_drag_x(position.x + size.width / 2.0);
    state.set_drag_y(position.y + size.height / 2.0);
    settle();
    assert_eq!(state.get_drop_list_id(), "l1");

    // Moving off the row gives it up again, so a drop elsewhere is not misread.
    state.set_drag_y(position.y - size.height);
    settle();
    assert_eq!(state.get_drop_list_id(), "");

    // A task from another project cannot land here at all.
    state.set_dragging_project_id(SharedString::from("p2"));
    state.set_drag_y(position.y + size.height / 2.0);
    settle();
    assert_eq!(state.get_drop_list_id(), "");

    state.set_dragging_task_id(SharedString::default());
    settle();
}

fn the_repeat_editor_reaches_its_handlers() {
    let window = window_with_content();
    let state = window.global::<AppState>();
    let task = state
        .get_tasks()
        .row_data(0)
        .expect("the test task should exist");
    state.set_selected_task_id(task.id.clone());
    state.set_selected_task(task);
    state.set_has_selected_task(true);

    // The real handler resolves the task's rule before opening; the test stands
    // in for it so the dialog has a draft to edit.
    let opened = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let opened = Rc::clone(&opened);
        let weak = window.as_weak();
        window
            .global::<Actions>()
            .on_open_recurrence(move |task_id| {
                opened.borrow_mut().push(task_id.to_string());
                let window = weak.upgrade().expect("the window outlives the test");
                let state = window.global::<AppState>();
                state.set_recurrence(recurrence_state(task_id.as_str()));
                state.set_recurrence_open(true);
            });
    }

    let previewed = Rc::new(RefCell::new(Vec::<RecurrenceState>::new()));
    {
        let previewed = Rc::clone(&previewed);
        window
            .global::<Actions>()
            .on_preview_recurrence(move |draft| previewed.borrow_mut().push(draft));
    }

    let saved = Rc::new(RefCell::new(Vec::<RecurrenceState>::new()));
    {
        let saved = Rc::clone(&saved);
        window
            .global::<Actions>()
            .on_save_recurrence(move |draft| saved.borrow_mut().push(draft));
    }

    assert!(
        activate(&window, "Edit the repeat schedule"),
        "the repeat row is not reachable from the detail pane"
    );
    assert_eq!(opened.borrow().as_slice(), ["t1"]);
    settle();

    assert!(
        activate(&window, "Tuesday"),
        "the weekday toggles are not reachable: {:?}",
        accessible_labels(&window)
    );
    settle();
    assert!(
        previewed
            .borrow()
            .last()
            .expect("editing should ask for a preview")
            .tuesday,
        "the toggled weekday did not reach the preview"
    );

    assert!(
        activate(&window, "Month"),
        "the period buttons are not reachable"
    );
    settle();
    assert_eq!(
        previewed
            .borrow()
            .last()
            .expect("changing the period should ask for a preview")
            .unit,
        RecurrenceUnit::Month
    );

    assert!(
        activate(&window, "Save"),
        "the save button is not reachable"
    );
    let saved = saved.borrow();
    let draft = saved.last().expect("saving should report the draft");
    assert_eq!(draft.task_id, "t1");
    assert_eq!(draft.unit, RecurrenceUnit::Month);
    assert!(draft.tuesday);
    assert!(
        !window.global::<AppState>().get_recurrence_open(),
        "saving should close the editor"
    );
}

fn the_repeat_editor_can_stop_a_schedule() {
    let window = window_with_content();
    let state = window.global::<AppState>();
    state.set_recurrence(recurrence_state("t1"));
    state.set_recurrence_open(true);
    settle();

    let cleared = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let cleared = Rc::clone(&cleared);
        window
            .global::<Actions>()
            .on_clear_recurrence(move |task_id| cleared.borrow_mut().push(task_id.to_string()));
    }

    assert!(
        activate(&window, "Stop repeating"),
        "the stop-repeating button is not reachable: {:?}",
        accessible_labels(&window)
    );
    assert_eq!(cleared.borrow().as_slice(), ["t1"]);
    assert!(!window.global::<AppState>().get_recurrence_open());
}

/// The subtask row used to be a label with a click target and nothing else: no
/// way to rename, delete, or complete a step without opening its parent.
fn subtask_rows_reach_their_handlers() {
    let window = window_with_content();
    let state = window.global::<AppState>();
    let mut task = state
        .get_tasks()
        .row_data(0)
        .expect("the test task should exist");
    task.subtasks = ModelRc::new(VecModel::from(vec![subtask_item("s1", "Pick up bread")]));
    state.set_tasks(ModelRc::new(VecModel::from(vec![task.clone()])));
    state.set_selected_task_id(task.id.clone());
    state.set_selected_task(task);
    state.set_has_selected_task(true);

    let toggled = Rc::new(RefCell::new(Vec::<String>::new()));
    let deleted = Rc::new(RefCell::new(Vec::<String>::new()));
    let opened = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let toggled = Rc::clone(&toggled);
        let deleted = Rc::clone(&deleted);
        let opened = Rc::clone(&opened);
        let actions = window.global::<Actions>();
        actions.on_toggle_subtask_completed(move |id| toggled.borrow_mut().push(id.to_string()));
        actions.on_delete_subtask(move |id| deleted.borrow_mut().push(id.to_string()));
        actions.on_select_subtask(move |id| opened.borrow_mut().push(id.to_string()));
    }

    assert!(
        activate(&window, "Complete Pick up bread"),
        "the subtask checkbox is not reachable: {:?}",
        accessible_labels(&window)
    );
    assert!(activate(&window, "Delete subtask Pick up bread"));
    assert!(activate(&window, "Open subtask Pick up bread"));

    assert_eq!(toggled.borrow().as_slice(), ["s1"]);
    assert_eq!(deleted.borrow().as_slice(), ["s1"]);
    assert_eq!(opened.borrow().as_slice(), ["s1"]);
}

/// Selecting a subtask published an id that no view read, so the detail pane
/// kept showing the parent task.
fn the_subtask_detail_pane_reaches_its_handlers() {
    let window = window_with_content();
    let state = window.global::<AppState>();
    let mut task = state
        .get_tasks()
        .row_data(0)
        .expect("the test task should exist");
    let subtask = subtask_item("s1", "Pick up bread");
    task.subtasks = ModelRc::new(VecModel::from(vec![subtask.clone()]));
    state.set_selected_task_id(task.id.clone());
    state.set_selected_task(task);
    state.set_has_selected_task(true);
    state.set_selected_subtask_id(subtask.id.clone());
    state.set_selected_subtask(subtask);
    state.set_has_selected_subtask(true);

    let titles = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
    let statuses = Rc::new(RefCell::new(Vec::<(String, TaskStatus)>::new()));
    let priorities = Rc::new(RefCell::new(Vec::<(String, TaskPriority)>::new()));
    let back = Rc::new(RefCell::new(0));
    {
        let titles = Rc::clone(&titles);
        let statuses = Rc::clone(&statuses);
        let priorities = Rc::clone(&priorities);
        let back = Rc::clone(&back);
        let actions = window.global::<Actions>();
        actions.on_update_subtask_title(move |id, title| {
            titles
                .borrow_mut()
                .push((id.to_string(), title.to_string()));
        });
        actions.on_update_subtask_status(move |id, status| {
            statuses.borrow_mut().push((id.to_string(), status));
        });
        actions.on_update_subtask_priority(move |id, priority| {
            priorities.borrow_mut().push((id.to_string(), priority));
        });
        actions.on_go_to_parent_task(move || *back.borrow_mut() += 1);
    }

    set_value(&window, "Subtask title", "Pick up rye bread");
    assert!(activate(&window, "Change status"));
    settle();
    assert!(activate(&window, "Change priority"));
    settle();
    assert!(
        activate(&window, "Back to Buy milk"),
        "the parent-task link is not reachable: {:?}",
        accessible_labels(&window)
    );

    assert_eq!(
        titles.borrow().as_slice(),
        [("s1".to_string(), "Pick up rye bread".to_string())]
    );
    // The status and priority options are inside a popup, out of reach here;
    // opening the fields is as far as this test can drive them.
    assert!(statuses.borrow().is_empty());
    assert!(priorities.borrow().is_empty());
    assert_eq!(*back.borrow(), 1);
}

/// The task detail pane offered only a due date, although the design and the
/// model both carry a start date for tasks that span a period.
fn the_start_date_editor_reaches_its_handlers() {
    let window = window_with_content();
    let state = window.global::<AppState>();
    let task = state
        .get_tasks()
        .row_data(0)
        .expect("the test task should exist");
    state.set_selected_task_id(task.id.clone());
    state.set_selected_task(task);
    state.set_has_selected_task(true);

    let ranged = Rc::new(RefCell::new(Vec::<(String, bool)>::new()));
    {
        let ranged = Rc::clone(&ranged);
        window
            .global::<Actions>()
            .on_set_task_range(move |task_id, is_range| {
                ranged.borrow_mut().push((task_id.to_string(), is_range));
            });
    }

    // A task without a range shows the deadline alone.
    assert!(
        ElementHandle::find_by_accessible_label(&window, "Edit start date")
            .next()
            .is_none(),
        "the start-date picker must stay hidden until the range is on"
    );
    assert!(activate(&window, "Use a start and due date"));
    assert_eq!(ranged.borrow().as_slice(), [("t1".to_string(), true)]);

    let mut ranged_task = state.get_selected_task();
    ranged_task.is_range_date = true;
    // The clear affordance only appears once there is a value to clear.
    ranged_task.has_start = true;
    ranged_task.start_label = SharedString::from("2026-09-06 09:00");
    state.set_selected_task(ranged_task);
    settle();

    let cleared = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let cleared = Rc::clone(&cleared);
        window
            .global::<Actions>()
            .on_clear_task_start(move |task_id| cleared.borrow_mut().push(task_id.to_string()));
    }
    assert!(
        ElementHandle::find_by_accessible_label(&window, "Edit start date")
            .next()
            .is_some(),
        "the start-date picker is not reachable: {:?}",
        accessible_labels(&window)
    );
    assert!(activate(&window, "Clear start date"));
    assert_eq!(cleared.borrow().as_slice(), ["t1"]);
}

/// The language switch had a Rust handler but no control anywhere in the UI.
fn the_language_switch_reaches_its_handler() {
    let window = window_with_content();
    window.global::<SettingsState>().set_open(true);
    window
        .global::<SettingsState>()
        .set_selected_category(SettingsCategory::Basic);
    settle();

    let chosen = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let chosen = Rc::clone(&chosen);
        window
            .global::<Actions>()
            .on_update_language(move |locale| chosen.borrow_mut().push(locale.to_string()));
    }

    assert!(
        activate(&window, "日本語"),
        "the language choices are not reachable: {:?}",
        accessible_labels(&window)
    );
    assert_eq!(chosen.borrow().as_slice(), ["ja"]);
}

/// Recurrence presets are a settings-owned list the repeat editor reads, so
/// both ends are exercised here.
fn recurrence_presets_reach_their_handlers() {
    let window = window_with_content();
    let settings = window.global::<SettingsState>();
    settings.set_open(true);
    settings.set_selected_category(SettingsCategory::Basic);
    settings.set_recurrence_presets(ModelRc::new(VecModel::from(vec![
        RecurrencePresetSetting {
            interval: 2,
            unit: RecurrenceUnit::Week,
        },
    ])));
    settle();

    let added = Rc::new(RefCell::new(Vec::<(i32, RecurrenceUnit)>::new()));
    let removed = Rc::new(RefCell::new(Vec::<(i32, RecurrenceUnit)>::new()));
    {
        let added = Rc::clone(&added);
        let removed = Rc::clone(&removed);
        let actions = window.global::<Actions>();
        actions.on_add_recurrence_preset(move |interval, unit| {
            added.borrow_mut().push((interval, unit));
        });
        actions.on_remove_recurrence_preset(move |interval, unit| {
            removed.borrow_mut().push((interval, unit));
        });
    }

    assert!(
        activate(&window, "Add recurrence preset"),
        "the preset controls are not reachable: {:?}",
        accessible_labels(&window)
    );
    assert!(activate(&window, "Remove the Every 2 weeks preset"));
    assert_eq!(added.borrow().as_slice(), [(2, RecurrenceUnit::Week)]);
    assert_eq!(removed.borrow().as_slice(), [(2, RecurrenceUnit::Week)]);

    settings.set_open(false);
    settle();

    // The same preset applies the unit and the interval in the repeat editor.
    let state = window.global::<AppState>();
    state.set_recurrence(recurrence_state("t1"));
    state.set_recurrence_open(true);
    settle();

    assert!(
        activate(&window, "Every 2 weeks"),
        "the repeat editor does not offer the saved presets: {:?}",
        accessible_labels(&window)
    );
    state.set_recurrence_open(false);
}

/// The font setting listed three hard-coded names; it now filters whatever the
/// platform reported.
fn the_font_picker_lists_what_the_platform_reported() {
    let window = window_with_content();
    let settings = window.global::<SettingsState>();
    settings.set_open(true);
    settings.set_font_options(ModelRc::new(VecModel::from(vec![
        SharedString::from("default"),
        SharedString::from("system"),
        SharedString::from("DejaVu Sans"),
    ])));
    settle();
    // The pane scrolls to the chosen category, and the accessibility tree only
    // carries what is inside the viewport, so the jump is what brings the font
    // controls into reach.
    {
        let weak = window.as_weak();
        window
            .global::<Actions>()
            .on_select_settings_category(move |category| {
                if let Some(window) = weak.upgrade() {
                    window
                        .global::<SettingsState>()
                        .set_selected_category(category);
                }
            });
    }
    assert!(activate(&window, "Appearance"));
    settle();

    let queries = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let queries = Rc::clone(&queries);
        window
            .global::<Actions>()
            .on_filter_fonts(move |query| queries.borrow_mut().push(query.to_string()));
    }

    set_value(&window, "Font", "Dejavu");
    assert_eq!(queries.borrow().as_slice(), ["Dejavu"]);
    settings.set_open(false);
}

/// Tabs through an open dialog and reports whether focus ever left it.
///
/// Down is the probe because every dialog ignores it while the task list moves
/// its selection with it, so a recorded move means the focus escaped. The loop
/// runs far longer than any dialog's control count, so a leak that only shows
/// up after a full cycle is caught too.
fn focus_escapes_while_tabbing(window: &AppWindow, moved: &Rc<RefCell<Vec<i32>>>) -> bool {
    moved.borrow_mut().clear();
    for _ in 0..40 {
        press_key(window, '\t');
        press_key(window, slint::platform::Key::DownArrow.into());
    }
    !moved.borrow().is_empty()
}

/// A modal must not hand keyboard focus to the panes behind it.
///
/// Slint's Tab traversal walks the whole window, so without the sentinels
/// around a dialog the focus leaves it after a few presses and lands in the
/// task list — where the arrow keys move a selection the user cannot see and
/// Space completes a task while the dialog is still on screen.
fn a_modal_keeps_keyboard_focus_inside_itself() {
    let window = window_with_content();
    let moved = Rc::new(RefCell::new(Vec::<i32>::new()));
    {
        let moved = Rc::clone(&moved);
        window
            .global::<Actions>()
            .on_move_task_selection(move |delta| moved.borrow_mut().push(delta));
    }
    // Key events are only delivered to an active window.
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(true));
    // The task list only reacts to Space when a row is selected, so give it
    // something to react to: the assertion below has to be able to fail.
    window
        .global::<AppState>()
        .set_selected_task_id("t1".into());

    // Positive control: with focus in the list, Down moves the selection.
    // Without it the assertion below would also pass if key events stopped
    // arriving at all.
    window.global::<AppState>().set_focus_list_request(1);
    settle();
    press_key(&window, slint::platform::Key::DownArrow.into());
    assert_eq!(
        moved.borrow().as_slice(),
        [1],
        "the task list no longer moves its selection with the arrow keys"
    );
    moved.borrow_mut().clear();

    let app_state = window.global::<AppState>();

    assert!(activate(&window, "New project"));
    settle();
    assert!(
        !focus_escapes_while_tabbing(&window, &moved),
        "keyboard focus escaped the project editor"
    );
    app_state.set_editor_open(false);
    settle();

    // The delete confirmation replaces the control the focus was on, which is
    // where a trap is easiest to lose.
    assert!(activate(&window, "Edit My Tasks"));
    settle();
    assert!(activate(&window, "Delete"));
    settle();
    assert!(
        !focus_escapes_while_tabbing(&window, &moved),
        "keyboard focus escaped the delete confirmation"
    );
    app_state.set_editor_open(false);
    settle();

    app_state.set_tag_manager_open(true);
    settle();
    assert!(
        !focus_escapes_while_tabbing(&window, &moved),
        "keyboard focus escaped the tag manager"
    );
    app_state.set_tag_manager_open(false);
    settle();

    app_state.set_recurrence(recurrence_state("t1"));
    app_state.set_recurrence_open(true);
    settle();
    assert!(
        !focus_escapes_while_tabbing(&window, &moved),
        "keyboard focus escaped the repeat editor"
    );
    app_state.set_recurrence_open(false);
    settle();

    window.global::<SettingsState>().set_open(true);
    settle();
    assert!(
        !focus_escapes_while_tabbing(&window, &moved),
        "keyboard focus escaped the settings dialog"
    );
    window.global::<SettingsState>().set_open(false);
    settle();
}

/// A pointer click has to reach the control it lands on.
///
/// Every other case here activates controls through the accessibility tree,
/// which addresses an element directly and so cannot see geometry at all: a
/// control of zero size, or one buried under another `TouchArea`, passes those
/// tests and is still dead to the mouse. These cases send real pointer events
/// at the element's own centre instead, so the result depends on hit testing.
fn a_pointer_click_reaches_the_control_under_it() {
    let window = window_with_content();
    resize(&window, 1200.0, 800.0);
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let seen = Rc::clone(&seen);
        window
            .global::<Actions>()
            .on_select_project(move |id| seen.borrow_mut().push(id.to_string()));
    }

    assert!(
        click(&window, "My Tasks"),
        "the project row is not in the element tree"
    );
    assert_eq!(
        seen.borrow().as_slice(),
        ["p1"],
        "the project row does not respond to a click at its own centre"
    );
}

/// An open modal has to swallow the clicks aimed at what it covers.
///
/// The project editor is a small card in the middle of the window, so the
/// sidebar rows it covers are covered by its scrim alone — nothing else is
/// between them and the pointer. The shell keeps its `TouchArea`s while a
/// dialog is open, and Slint delivers a press to whatever is topmost, so a
/// scrim that lost its `TouchArea` would let the click through. It would still
/// look right and still trap the keyboard: only a pointer test notices.
fn an_open_dialog_absorbs_clicks_meant_for_the_shell() {
    let window = window_with_content();
    resize(&window, 1200.0, 800.0);
    let selected = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let selected = Rc::clone(&selected);
        window
            .global::<Actions>()
            .on_select_project(move |id| selected.borrow_mut().push(id.to_string()));
    }

    // Positive control: the row reacts to a click, so the assertion after the
    // dialog opens can fail.
    assert!(
        click(&window, "My Tasks"),
        "the project row is not reachable"
    );
    assert_eq!(selected.borrow().as_slice(), ["p1"]);
    selected.borrow_mut().clear();

    assert!(activate(&window, "New project"), "the editor does not open");
    settle();

    assert!(
        click(&window, "My Tasks"),
        "the project row left the element tree while the dialog was open"
    );
    assert!(
        selected.borrow().is_empty(),
        "a click passed through the dialog scrim and selected a project"
    );

    // The dialog's own controls must still be clickable, or the assertion above
    // would also hold for a dialog that swallows every click including its own.
    assert!(
        click(&window, "Cancel"),
        "the dialog's cancel button is not reachable"
    );
    assert!(
        !window.global::<AppState>().get_editor_open(),
        "the cancel button does not respond to a click"
    );
}

/// The compact sidebar is an overlay, so it has to cover the pane behind it.
///
/// It is drawn over the task list rather than beside it, and only its scrim
/// keeps a tap meant for the sidebar from reaching a task row underneath.
fn the_compact_sidebar_overlay_covers_the_task_list() {
    let window = window_with_content();
    resize(&window, 480.0, 800.0);
    let selected = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let selected = Rc::clone(&selected);
        window
            .global::<Actions>()
            .on_select_task(move |id| selected.borrow_mut().push(id.to_string()));
    }

    // Positive control: with the overlay closed the row is clickable, so the
    // assertion after it can fail.
    assert!(click(&window, "Buy milk"), "the task row is not reachable");
    assert_eq!(
        selected.borrow().as_slice(),
        ["t1"],
        "the compact task row does not respond to a tap"
    );
    selected.borrow_mut().clear();

    window.global::<AppState>().set_sidebar_open(true);
    settle();

    assert!(click(&window, "Buy milk"), "the task row left the tree");
    assert!(
        selected.borrow().is_empty(),
        "a tap passed through the sidebar overlay and selected a task"
    );

    window.global::<AppState>().set_sidebar_open(false);
    settle();
}

/// The loading veil has to swallow input, not merely dim the shell.
///
/// It goes up while a reload is in flight, when the models behind the rows are
/// about to be replaced; a click that gets through addresses a row that is on
/// its way out.
fn the_loading_veil_swallows_clicks() {
    let window = window_with_content();
    resize(&window, 1200.0, 800.0);
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    {
        let seen = Rc::clone(&seen);
        window
            .global::<Actions>()
            .on_select_project(move |id| seen.borrow_mut().push(id.to_string()));
    }

    window.global::<AppState>().set_loading(true);
    settle();

    assert!(
        click(&window, "My Tasks"),
        "the project row left the element tree while loading"
    );
    assert!(
        seen.borrow().is_empty(),
        "a click passed through the loading veil"
    );
}

/// Growing the task list must not grow what the UI builds.
///
/// The pane is a `ListView`, which instantiates only the rows inside its
/// viewport; a plain layout would build one row per task and make a large
/// project unusable. Counting instantiated rows is what tells the two apart.
fn a_long_task_list_only_instantiates_visible_rows() {
    let window = window_with_content();

    let instantiated = |count: usize| {
        let tasks: Vec<TaskItem> = (0..count)
            .map(|index| task_item(&format!("t{index}"), &format!("Task {index}")))
            .collect();
        window
            .global::<AppState>()
            .set_tasks(ModelRc::new(VecModel::from(tasks)));
        settle();
        i_slint_backend_testing::ElementQuery::from_root(&window)
            .match_descendants()
            .match_accessible_role(i_slint_backend_testing::AccessibleRole::ListItem)
            .find_all()
            .len()
    };

    let small = instantiated(500);
    let large = instantiated(5_000);

    assert!(small > 0, "no task row was instantiated at all");
    // Ten times the tasks must not mean ten times the rows: the count is
    // bounded by the viewport, not by the model.
    assert!(
        large <= small,
        "{small} rows for 500 tasks but {large} for 5000; the list is no longer virtualised"
    );
    assert!(
        small < 500,
        "every one of the 500 tasks was instantiated; the list is no longer virtualised"
    );
}

/// Slint's backend is process-global and its components are `!Send`, so every
/// case runs inside one test on one thread.
#[test]
fn the_shell_responds_to_user_actions() {
    selecting_a_project_reaches_its_handler();
    selecting_a_task_list_reaches_its_handler();
    a_due_filter_reaches_its_handler();
    a_task_row_can_be_selected_and_completed();
    a_task_row_shows_its_tags();
    tag_management_and_assignment_reach_their_handlers();
    the_sort_bar_reaches_its_handler();
    the_drag_handle_reaches_its_handler();
    a_sidebar_list_becomes_a_drop_target();
    expanding_a_project_reaches_its_handler();
    a_search_suggestion_reaches_its_handler();
    project_and_task_list_management_reaches_its_handlers();
    the_settings_dialog_reaches_its_handlers();
    the_account_menu_reaches_its_destinations();
    task_list_empty_states_offer_a_next_action();
    notification_permission_can_be_requested_from_settings();
    adding_a_subtask_reaches_its_handler();
    the_due_date_editor_is_reachable();
    reminder_controls_reach_their_handlers();
    the_priority_editor_reaches_its_handler();
    the_status_editor_reaches_its_handler();
    the_expanded_sidebar_can_be_collapsed_and_reopened();
    the_medium_sidebar_stays_narrow_without_a_toggle();
    the_compact_sidebar_opens_and_closes_as_an_overlay();
    the_repeat_editor_reaches_its_handlers();
    the_repeat_editor_can_stop_a_schedule();
    the_theme_switches_between_light_and_dark();
    subtask_rows_reach_their_handlers();
    the_subtask_detail_pane_reaches_its_handlers();
    the_start_date_editor_reaches_its_handlers();
    the_language_switch_reaches_its_handler();
    recurrence_presets_reach_their_handlers();
    the_font_picker_lists_what_the_platform_reported();
    a_modal_keeps_keyboard_focus_inside_itself();
    a_pointer_click_reaches_the_control_under_it();
    an_open_dialog_absorbs_clicks_meant_for_the_shell();
    the_compact_sidebar_overlay_covers_the_task_list();
    the_loading_veil_swallows_clicks();
    a_long_task_list_only_instantiates_visible_rows();
}

/// The colour tokens once ignored `Theme.mode`: nothing derived `Theme.dark`
/// from it, so every surface stayed light whatever the settings dialog said.
fn the_theme_switches_between_light_and_dark() {
    let window = window_with_content();
    let theme = window.global::<Theme>();

    // `AppWindow.init` samples the OS scheme; pin it so the assertions do not
    // depend on the appearance of the machine running the tests.
    theme.set_system_dark(false);

    theme.set_mode(ThemeMode::Light);
    assert!(!theme.get_dark());
    let light_background = theme.get_background();

    theme.set_mode(ThemeMode::Dark);
    assert!(theme.get_dark());
    assert_ne!(
        theme.get_background(),
        light_background,
        "the dark theme must repaint the surfaces"
    );

    theme.set_mode(ThemeMode::System);
    assert!(!theme.get_dark());
    theme.set_system_dark(true);
    assert!(theme.get_dark(), "system mode must follow the OS scheme");

    theme.set_font_color_choice("white".into());
    assert_eq!(
        theme.get_text(),
        Color::from_rgb_u8(0xff, 0xff, 0xff).into()
    );
    theme.set_background_color_choice("black".into());
    assert_eq!(
        theme.get_background(),
        Color::from_rgb_u8(0x00, 0x00, 0x00).into()
    );

    theme.set_mode(ThemeMode::Light);
    theme.set_font_color_choice("#000000".into());
    theme.set_background_color_choice("#FFFFFF".into());
    assert_eq!(
        theme.get_text(),
        Color::from_rgb_u8(0x21, 0x25, 0x29).into(),
        "legacy defaults must keep following the selected theme"
    );
    assert_eq!(
        theme.get_background(),
        Color::from_rgb_u8(0xff, 0xff, 0xff).into()
    );
}
