mod support;

use ers::annotations::TextAnchor;
use ers::configuration::{AppPaths, Command, Configuration, InputKey, PathEnvironment};
use ers::document::SourceDocument;
use ers::export::{
    ExportOptions, MergeRequest, export_book, merge_section, render_section, validate_destination,
};
use ers::storage::Store;
use rusqlite::Connection;
use serde::Serialize;
use support::{annotation, book, library_entry, link, reading_state};
use tempfile::tempdir;

#[test]
fn terminal_cell_widths_match_python() {
    let fixtures =
        serde_json::from_str::<support::FixtureBundle>(include_str!("fixtures/core.json")).unwrap();
    for fixture in fixtures.cells {
        assert_eq!(
            ers::document::text::cell_width(&fixture.text),
            fixture.width,
            "{:?}",
            fixture.text
        );
        assert_eq!(
            ers::document::text::graphemes(&fixture.text),
            fixture.graphemes,
            "{:?}",
            fixture.text
        );
    }
}

#[test]
fn wrapped_text_matches_expected_rows() {
    let fixtures =
        serde_json::from_str::<support::FixtureBundle>(include_str!("fixtures/core.json")).unwrap();
    for fixture in fixtures.wrapping {
        assert_eq!(
            ers::document::text::wrap_text(&fixture.text, fixture.width),
            fixture.lines,
            "width {}: {:?}",
            fixture.width,
            fixture.text
        );
    }
}

#[test]
fn python_document_fixtures_preserve_canonical_offsets() {
    let fixtures =
        serde_json::from_str::<support::FixtureBundle>(include_str!("fixtures/core.json")).unwrap();
    for fixture in fixtures.documents {
        let parsed = ers::document::text::parse_document("chapter.xhtml", &fixture.html).unwrap();
        assert_eq!(
            parsed
                .paragraphs
                .iter()
                .map(|paragraph| paragraph.text.clone())
                .collect::<Vec<_>>(),
            fixture.paragraphs,
            "{}",
            fixture.name
        );
        assert_eq!(parsed.source.text, fixture.canonical, "{}", fixture.name);
    }
}

#[test]
fn python_export_fixtures_match_exact_bytes() {
    let fixtures =
        serde_json::from_str::<support::FixtureBundle>(include_str!("fixtures/core.json")).unwrap();
    for fixture in fixtures.exports {
        let section = render_section(&fixture.book, &fixture.annotations, &fixture.newline);
        assert_eq!(section, fixture.section.as_bytes(), "{}", fixture.name);
        let result = merge_section(MergeRequest {
            content: fixture.original.as_bytes(),
            book_id: &fixture.book.id,
            section: &section,
            expected_digest: fixture.expected_digest.as_deref(),
            overwrite: fixture.overwrite,
        })
        .unwrap();
        assert_eq!(
            result.content,
            fixture.merged.as_bytes(),
            "{}",
            fixture.name
        );
        assert_eq!(result.digest, fixture.digest, "{}", fixture.name);
    }
}

#[test]
fn canceled_export_leaves_the_destination_unchanged() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("notes.md");
    std::fs::write(&path, "Handwritten notes").unwrap();
    let mut store = Store::open_in_memory().unwrap();
    store
        .identify_book(&book(), "/synthetic/book.epub")
        .unwrap();
    store.set_link(&link(path.to_str().unwrap())).unwrap();
    store.save_annotation(&annotation()).unwrap();
    let canceled = std::sync::atomic::AtomicBool::new(true);
    let result = export_book(
        &mut store,
        &book(),
        ExportOptions {
            canceled: Some(&canceled),
            ..ExportOptions::default()
        },
    );
    assert!(matches!(result, Err(ers::Error::Canceled)));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "Handwritten notes");
    assert_eq!(
        store
            .export_link(&book().id)
            .unwrap()
            .unwrap()
            .exported_revision,
        -1
    );
}

#[test]
fn filesystem_export_preserves_handwritten_bytes() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("My notes.md");
    let before = b"Handwritten prefix\r\nNo final newline";
    std::fs::write(&path, before).unwrap();
    let mut store = Store::open_in_memory().unwrap();
    store
        .identify_book(&book(), "/synthetic/book.epub")
        .unwrap();
    store.set_link(&link(path.to_str().unwrap())).unwrap();
    store.save_annotation(&annotation()).unwrap();
    let digest = export_book(&mut store, &book(), ExportOptions::default())
        .unwrap()
        .unwrap();
    let exported = std::fs::read(&path).unwrap();
    assert!(exported.starts_with(before));
    assert!(exported.windows(2).any(|bytes| bytes == b"\r\n"));
    assert_eq!(
        std::fs::read(directory.path().join(".My notes.md.ers.bak")).unwrap(),
        before
    );
    assert_eq!(
        store.export_link(&book().id).unwrap().unwrap().digest,
        Some(digest.digest)
    );
    export_book(&mut store, &book(), ExportOptions::default()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), exported);
    assert_eq!(
        std::fs::read(directory.path().join(".My notes.md.ers.bak")).unwrap(),
        before
    );
}

#[test]
fn failed_export_keeps_the_annotation_durable() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("vanished.md");
    let mut store = Store::open_in_memory().unwrap();
    store
        .identify_book(&book(), "/synthetic/book.epub")
        .unwrap();
    store.set_link(&link(path.to_str().unwrap())).unwrap();
    store.save_annotation(&annotation()).unwrap();
    assert!(export_book(&mut store, &book(), ExportOptions::default()).is_err());
    assert!(!path.exists());
    assert_eq!(store.annotations(&book().id).unwrap(), vec![annotation()]);
    assert_eq!(
        store
            .export_link(&book().id)
            .unwrap()
            .unwrap()
            .exported_revision,
        -1
    );
}

#[test]
fn invalid_destinations_are_rejected() {
    for destination in [
        "relative.md",
        "/not-a-markdown.txt",
        "/notes.md\nsecond.md",
        "/notes\0.md",
    ] {
        assert!(validate_destination(destination).is_err());
    }
}

#[cfg(unix)]
#[test]
fn symlink_destination_preserves_the_link() {
    let directory = tempdir().unwrap();
    let target = directory.path().join("target.md");
    let symlink = directory.path().join("linked.md");
    std::fs::write(&target, b"Handwritten").unwrap();
    std::os::unix::fs::symlink(&target, &symlink).unwrap();
    let mut store = Store::open_in_memory().unwrap();
    store
        .identify_book(&book(), "/synthetic/book.epub")
        .unwrap();
    store.set_link(&link(symlink.to_str().unwrap())).unwrap();
    store.save_annotation(&annotation()).unwrap();
    export_book(&mut store, &book(), ExportOptions::default()).unwrap();
    assert!(symlink.is_symlink());
    assert!(std::fs::read(&target).unwrap().starts_with(b"Handwritten"));
}

#[cfg(unix)]
#[test]
fn export_retains_destination_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().unwrap();
    let path = directory.path().join("private.md");
    std::fs::write(&path, b"Handwritten").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    let mut store = Store::open_in_memory().unwrap();
    store
        .identify_book(&book(), "/synthetic/book.epub")
        .unwrap();
    store.set_link(&link(path.to_str().unwrap())).unwrap();
    export_book(&mut store, &book(), ExportOptions::default()).unwrap();
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o640
    );
}

#[test]
fn external_section_edits_require_confirmation() {
    let section = render_section(&book(), &[annotation()], "\n");
    let initial = merge_section(MergeRequest {
        content: b"Handwritten notes",
        book_id: &book().id,
        section: &section,
        expected_digest: None,
        overwrite: false,
    })
    .unwrap();
    let edited = String::from_utf8(initial.content)
        .unwrap()
        .replace("Personal note", "External edit");
    assert!(
        merge_section(MergeRequest {
            content: edited.as_bytes(),
            book_id: &book().id,
            section: &section,
            expected_digest: Some(&initial.digest),
            overwrite: false,
        })
        .is_err()
    );
}

#[test]
fn malformed_delimiters_remain_errors_when_forced() {
    let section = render_section(&book(), &[annotation()], "\n");
    let malformed = format!("<!-- ers:annotations:{}:broken -->", book().id);
    assert!(
        merge_section(MergeRequest {
            content: malformed.as_bytes(),
            book_id: &book().id,
            section: &section,
            expected_digest: None,
            overwrite: true,
        })
        .is_err()
    );
}

#[test]
fn missing_linked_section_is_not_silently_appended() {
    assert!(
        merge_section(MergeRequest {
            content: b"Only handwritten content",
            book_id: &book().id,
            section: &render_section(&book(), &[], "\n"),
            expected_digest: Some("previously-linked"),
            overwrite: true,
        })
        .is_err()
    );
}

#[test]
fn unicode_anchor_offsets_count_code_points() {
    let anchor = TextAnchor::new("chapter.xhtml", 2, "café 👩‍🔬").unwrap();
    assert_eq!(anchor.end(), 10);
    let document = SourceDocument::new("chapter.xhtml", "xxcafé 👩‍🔬 suffix");
    assert_eq!(anchor.resolve(&document), Some(&anchor));
}

#[test]
fn repeated_quote_never_relocates_an_anchor() {
    let anchor = TextAnchor::new("chapter.xhtml", 2, "quote").unwrap();
    let document = SourceDocument::new("chapter.xhtml", "changed text then quote");
    assert_eq!(anchor.resolve(&document), None);
}

#[test]
fn wrong_normalization_version_never_resolves() {
    let anchor = annotation().anchor;
    let mut document = SourceDocument::new("chapter.xhtml", "xxcafé 👩‍🔬 suffix");
    document.normalization_version = 2;
    assert_eq!(anchor.resolve(&document), None);
}

#[derive(Serialize)]
struct InvalidAnchor {
    content_id: String,
    start: i64,
    end: i64,
    quote: String,
}

#[test]
fn deserialization_rejects_invalid_anchor_ranges() {
    for (start, end, quote) in [(-1, 1, "ab"), (0, 0, ""), (0, 2, "é"), (3, 2, "a")] {
        let input = InvalidAnchor {
            content_id: "chapter.xhtml".into(),
            start,
            end,
            quote: quote.into(),
        };
        assert!(
            serde_json::from_str::<TextAnchor>(&serde_json::to_string(&input).unwrap()).is_err()
        );
    }
}

#[test]
fn application_paths_use_the_ers_configuration_directory() {
    let home = tempdir().unwrap();
    let environment = PathEnvironment {
        home: Some(home.path().into()),
        ..PathEnvironment::default()
    };
    let paths = AppPaths::discover(&environment).unwrap();
    assert_eq!(paths.database, home.path().join(".config/ers/states.db"));
    assert!(!paths.directory.exists());
}

#[test]
fn explicit_configuration_home_controls_storage() {
    let home = tempdir().unwrap();
    let config_home = tempdir().unwrap();
    let environment = PathEnvironment {
        home: Some(home.path().into()),
        config_home: Some(config_home.path().into()),
        ..PathEnvironment::default()
    };
    assert_eq!(
        AppPaths::discover(&environment).unwrap().directory,
        config_home.path().join("ers")
    );
}

#[test]
fn missing_home_returns_an_error_without_a_fallback_database() {
    assert!(AppPaths::discover(&PathEnvironment::default()).is_err());
}

#[test]
fn application_data_supplies_the_windows_directory() {
    let app_data = tempdir().unwrap();
    let environment = PathEnvironment {
        app_data: Some(app_data.path().into()),
        ..PathEnvironment::default()
    };
    assert_eq!(
        AppPaths::discover(&environment).unwrap().configuration,
        app_data.path().join("ers/configuration.json")
    );
}

#[test]
fn configuration_roundtrip_preserves_unknown_fields() {
    let mut config = Configuration::default();
    config
        .extra
        .insert("FutureFeature".into(), serde_json::Value::Bool(true));
    config.settings.extra.insert(
        "FutureSetting".into(),
        serde_json::Value::String("keep".into()),
    );
    let serialized = config.to_json().unwrap();
    let restored = Configuration::from_json(&serialized).unwrap();
    assert_eq!(restored, config);
}

#[test]
fn annotation_shortcuts_accept_unused_punctuation() {
    let mut config = Configuration::default();
    config.bindings.set(Command::AnnotationSelect, "!");
    let restored = Configuration::from_json(&config.to_json().unwrap()).unwrap();
    assert_eq!(
        restored.bindings.command_for(InputKey::Character('!')),
        Some(Command::AnnotationSelect)
    );
}

#[test]
fn annotation_binding_conflicts_are_rejected() {
    let mut config = Configuration::default();
    config.bindings.set(Command::AnnotationSelect, "j");
    assert!(Configuration::from_json(&config.to_json().unwrap()).is_err());
}

#[test]
fn builtin_navigation_binding_conflicts_are_rejected() {
    let mut config = Configuration::default();
    config.bindings.set(Command::JumpBackward, "\t");
    assert!(Configuration::from_json(&config.to_json().unwrap()).is_err());
}

#[test]
fn invalid_annotation_settings_are_rejected() {
    for (width, scrolloff) in [(15, 0), (101, 0), (36, -1)] {
        let mut config = Configuration::default();
        config.settings.annotation_notes_width = width;
        config.settings.annotation_scrolloff = scrolloff;
        assert!(Configuration::from_json(&config.to_json().unwrap()).is_err());
    }
}

#[test]
fn failed_session_save_rolls_back_the_reading_position() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("states.db");
    let path = "/synthetic/book.epub";
    let mut store = Store::open(&database).unwrap();
    store
        .save_session(&library_entry(path), &reading_state())
        .unwrap();
    let connection = Connection::open(&database).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_library BEFORE UPDATE ON library BEGIN SELECT RAISE(ABORT, 'rejected library write'); END;").unwrap();
    let mut moved = reading_state();
    moved.offset += 10;
    assert!(store.save_session(&library_entry(path), &moved).is_err());
    assert_eq!(store.reading_state(path).unwrap(), Some(reading_state()));
}

#[test]
fn first_bookmark_creates_its_required_reading_record() {
    let mut store = Store::open_in_memory().unwrap();
    let path = "/synthetic/new-book.epub";
    store
        .insert_bookmark(path, "First bookmark", &reading_state())
        .unwrap();
    assert_eq!(store.reading_state(path).unwrap(), Some(reading_state()));
    assert_eq!(store.bookmarks(path).unwrap()[0].position, reading_state());
}

#[test]
fn reading_state_update_preserves_bookmarks() {
    let mut store = Store::open_in_memory().unwrap();
    let path = "/synthetic/book.epub";
    store.save_reading_state(path, &reading_state()).unwrap();
    store
        .insert_bookmark(path, "My bookmark", &reading_state())
        .unwrap();
    let before = store.bookmarks(path).unwrap();
    let mut moved = reading_state();
    moved.offset += 10;
    store.save_reading_state(path, &moved).unwrap();
    assert_eq!(store.bookmarks(path).unwrap(), before);
    assert_eq!(store.reading_state(path).unwrap(), Some(moved));
}

#[test]
fn deleting_history_retains_annotation_ownership() {
    let mut store = Store::open_in_memory().unwrap();
    let path = "/synthetic/book.epub";
    store.identify_book(&book(), path).unwrap();
    store.save_reading_state(path, &reading_state()).unwrap();
    store.save_library_entry(&library_entry(path)).unwrap();
    store
        .insert_bookmark(path, "My bookmark", &reading_state())
        .unwrap();
    store.save_annotation(&annotation()).unwrap();
    store.set_link(&link("/synthetic/notes.md")).unwrap();
    store.delete_history(path).unwrap();
    assert!(store.history().unwrap().is_empty());
    assert!(store.bookmarks(path).unwrap().is_empty());
    assert_eq!(store.annotations(&book().id).unwrap(), vec![annotation()]);
    assert_eq!(
        store.export_link(&book().id).unwrap(),
        Some(link("/synthetic/notes.md"))
    );
}

#[test]
fn fingerprint_identity_survives_a_moved_book() {
    let mut store = Store::open_in_memory().unwrap();
    let original = store
        .identify_book(&book(), "/synthetic/first.epub")
        .unwrap();
    let mut proposed = book();
    proposed.id = "different-proposed-id".into();
    let moved = store
        .identify_book(&proposed, "/synthetic/moved.epub")
        .unwrap();
    assert_eq!(moved, original);
    assert_eq!(store.books().unwrap(), vec![original]);
}

#[test]
fn late_export_completion_does_not_replace_a_newer_digest() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .identify_book(&book(), "/synthetic/book.epub")
        .unwrap();
    store.set_link(&link("/synthetic/notes.md")).unwrap();
    store.save_annotation(&annotation()).unwrap();
    let first = store.export_link(&book().id).unwrap().unwrap();
    store.mark_exported(&first, "first-digest").unwrap();
    let mut edited = annotation();
    edited.note = "Newer note".into();
    store.save_annotation(&edited).unwrap();
    let second = store.export_link(&book().id).unwrap().unwrap();
    store.mark_exported(&second, "second-digest").unwrap();
    let completed = store.export_link(&book().id).unwrap();
    store.mark_exported(&first, "first-digest").unwrap();
    assert_eq!(store.export_link(&book().id).unwrap(), completed);
}

#[test]
fn export_completion_does_not_acknowledge_later_edits() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .identify_book(&book(), "/synthetic/book.epub")
        .unwrap();
    store.set_link(&link("/synthetic/notes.md")).unwrap();
    store.save_annotation(&annotation()).unwrap();
    let exporting = store.export_link(&book().id).unwrap().unwrap();
    let mut updated = annotation();
    updated.note = "A later edit".into();
    store.save_annotation(&updated).unwrap();
    store.mark_exported(&exporting, "digest").unwrap();
    let current = store.export_link(&book().id).unwrap().unwrap();
    assert_eq!((current.revision, current.exported_revision), (2, 1));
}

#[test]
fn newer_database_schema_is_rejected_without_mutation() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("states.db");
    let connection = Connection::open(&path).unwrap();
    connection.pragma_update(None, "user_version", 2).unwrap();
    drop(connection);
    let before = std::fs::read(&path).unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn unsupported_database_schema_is_rejected_without_mutation() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("states.db");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("CREATE TABLE unrelated (value TEXT);")
        .unwrap();
    drop(connection);
    let before = std::fs::read(&path).unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
