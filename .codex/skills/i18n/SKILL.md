---
name: i18n
description: Implement and maintain internationalization in Flequit with Slint bundled translations, including @tr() usage, .po workflow, runtime language switching, and Rust-side message handling.
---

# i18n Skill

Use this skill when adding or updating localized text.

## Mechanism

Slint bundled translations (`@tr()` + gettext `.po`, embedded at build time).
Runtime switching via `slint::select_bundled_translation(lang)`.
`update_all_translations()` is **not** needed with bundled translations.

## Core files

- `i18n/flequit-ui.pot` — extraction template (generated)
- `i18n/en/LC_MESSAGES/flequit-ui.po`
- `i18n/ja/LC_MESSAGES/flequit-ui.po`
- `crates/flequit-ui/build.rs` — bundles translations

## Syntax

```slint
@tr("Task title")                                  // basic
@tr("Welcome, {0}!", user-name)                    // arguments
@tr("{n} task" | "{n} tasks" % task-count)         // plural
@tr("Sidebar" => "Today")                          // context disambiguation
```

## Rust-side text

`@tr()` only works inside `.slint`. **Never build user-facing strings in Rust.**

- Rust passes an identifier (error code, status enum) to a property
- `ui/globals/i18n.slint` maps the identifier to `@tr()` text via a pure function
- Variable parts are passed as separate properties and interpolated in `.slint`

This keeps `.po` the single translation source. Log messages (`tracing`) stay English.

## Workflow

1. Add or edit `@tr("...")` in `.slint`
2. Regenerate the template:
   ```sh
   find crates/flequit-ui/ui -name '*.slint' | xargs slint-tr-extractor -o i18n/flequit-ui.pot
   ```
3. Merge into each locale:
   ```sh
   msgmerge --update i18n/ja/LC_MESSAGES/flequit-ui.po i18n/flequit-ui.pot
   ```
4. Translate the new entries
5. `cargo build` re-bundles automatically
6. Verify layout in both `en` and `ja`

Check completeness: `msgfmt --statistics -o /dev/null i18n/ja/LC_MESSAGES/flequit-ui.po`

## Language switching

1. Receive `I18n.locale-changed`
2. `slint::select_bundled_translation(&locale)`
3. Persist via `flequit-settings`
4. Update `I18n.current-locale`

At startup, call `select_bundled_translation()` **after** the first component is
created; earlier calls have no effect. Fall back to `en` for unsupported locales.

## Message authoring rules

- Write complete sentences/phrases. Never concatenate words into a sentence
  (word order differs across languages).
- Always use placeholders for variable parts.
- Add context (`"Context" =>`) for single words, abbreviations, and labels
  reused with different meanings.
- Changing the source text changes the msgid and drops the existing translation —
  update the `.po` entries when rewording.

Details: `docs/ja/develop/design/ui/i18n-system.md`.
