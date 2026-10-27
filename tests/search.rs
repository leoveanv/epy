use ers::cancellation::Cancellation;
use ers::document::RenderedDocument;
use ers::document::text::parse_document;
use ers::search::{SearchLimits, SearchPattern};

fn document(body: &str, width: usize) -> RenderedDocument {
    RenderedDocument::new(0, parse_document("chapter.xhtml", body).unwrap(), width)
}

#[test]
fn regex_search_matches_case_insensitive_unicode_with_canonical_offsets() {
    let document = document("<p>Café 👩‍🔬 CAFÉ</p>", 80);
    let matches = SearchPattern::compile("café", SearchLimits::default())
        .unwrap()
        .find(&document.layout, &Cancellation::default())
        .unwrap();
    assert_eq!(
        matches
            .hits
            .iter()
            .map(|hit| hit.offset)
            .collect::<Vec<_>>(),
        vec![0, 9]
    );
}

#[test]
fn regex_search_supports_lookaround_and_named_backreferences() {
    let document = document("<p>echo ECHO then other</p>", 80);
    // #NOTE: The pattern finds a repeated word after a word boundary, ignoring case.
    let pattern =
        SearchPattern::compile(r"(?<=\b)(?P<word>\w+)\s+(?P=word)", SearchLimits::default())
            .unwrap();
    assert_eq!(
        pattern
            .find(&document.layout, &Cancellation::default())
            .unwrap()
            .hits
            .len(),
        1
    );
}

#[test]
fn zero_width_matches_advance_without_losing_nonempty_alternatives() {
    let document = document("<p>aa</p>", 80);
    // #NOTE: The alternation permits an empty match immediately before each nonempty match.
    let pattern = SearchPattern::compile(r"(?=a)|a", SearchLimits::default()).unwrap();
    let matches = pattern
        .find(&document.layout, &Cancellation::default())
        .unwrap();
    assert_eq!(
        matches
            .hits
            .iter()
            .map(|hit| hit.offset)
            .collect::<Vec<_>>(),
        vec![0, 0, 1, 1]
    );
}

#[test]
fn search_uses_visible_citation_labels_without_changing_the_source() {
    let document = document("<p>Text<a epub:type='noteref' href='#n'>1</a></p>", 80);
    let original = document.parsed.source.clone();
    // #NOTE: The pattern matches the brackets added around the visible citation number.
    let pattern = SearchPattern::compile(r"\[1\]", SearchLimits::default()).unwrap();
    let matches = pattern
        .find(&document.layout, &Cancellation::default())
        .unwrap();
    assert_eq!(matches.hits.len(), 1);
    assert_eq!(matches.hits[0].offset, 4);
    assert_eq!(document.parsed.source, original);
}

#[test]
fn search_does_not_join_words_across_wrapped_lines() {
    let document = document("<p>alpha beta gamma delta</p>", 12);
    let pattern = SearchPattern::compile("beta gamma", SearchLimits::default()).unwrap();
    assert!(
        pattern
            .find(&document.layout, &Cancellation::default())
            .unwrap()
            .hits
            .is_empty()
    );
}

#[test]
fn case_insensitive_latin_ranges_include_dotted_and_dotless_i() {
    let document = document("<p>İ ı I i ſ K</p>", 80);
    // #NOTE: The Latin range includes Unicode case-equivalent letters under case-insensitive matching.
    let pattern = SearchPattern::compile("[a-z]+", SearchLimits::default()).unwrap();
    assert_eq!(
        pattern
            .find(&document.layout, &Cancellation::default())
            .unwrap()
            .hits
            .len(),
        6
    );
}

#[test]
fn literal_i_searches_include_all_four_case_equivalents() {
    let document = document("<p>İ ı I i</p>", 80);
    // #NOTE: The literal letter is searched without case sensitivity.
    let pattern = SearchPattern::compile("i", SearchLimits::default()).unwrap();
    assert_eq!(
        pattern
            .find(&document.layout, &Cancellation::default())
            .unwrap()
            .hits
            .len(),
        4
    );
}

#[test]
fn word_character_search_excludes_combining_marks() {
    let document = document("<p>é</p>", 80);
    // #NOTE: A word-only suffix cannot consume a trailing combining mark.
    let pattern = SearchPattern::compile(r"\w+$", SearchLimits::default()).unwrap();
    assert!(
        pattern
            .find(&document.layout, &Cancellation::default())
            .unwrap()
            .hits
            .is_empty()
    );
}

#[test]
fn word_character_search_does_not_include_other_connector_punctuation() {
    let document = document("<p>a‿b_</p>", 80);
    // #NOTE: Word characters include letters, numbers, and underscore, excluding undertie.
    let pattern = SearchPattern::compile(r"[\w]", SearchLimits::default()).unwrap();
    assert_eq!(
        pattern
            .find(&document.layout, &Cancellation::default())
            .unwrap()
            .hits
            .len(),
        3
    );
}

#[test]
fn nonword_boundaries_do_not_match_an_empty_line() {
    let document = document("<p>word</p>", 80);
    // #NOTE: A non-boundary assertion needs at least one neighboring character.
    let pattern = SearchPattern::compile(r"\B", SearchLimits::default()).unwrap();
    assert!(
        pattern
            .find(&document.layout, &Cancellation::default())
            .unwrap()
            .hits
            .iter()
            .all(|hit| !document.layout.text_lines[hit.row].is_empty())
    );
}

#[test]
fn unicode_character_escapes_match_their_code_points() {
    let document = document("<p>A 😀</p>", 80);
    // #NOTE: Four-digit and eight-digit Unicode escapes represent the displayed characters.
    for expression in [r"\u0041", r"\U0001F600"] {
        let pattern = SearchPattern::compile(expression, SearchLimits::default()).unwrap();
        assert_eq!(
            pattern
                .find(&document.layout, &Cancellation::default())
                .unwrap()
                .hits
                .len(),
            1
        );
    }
}

#[test]
fn named_unicode_escapes_match_the_named_character() {
    let document = document("<p>😀</p>", 80);
    // #NOTE: The named Unicode escape selects the grinning-face character.
    let pattern = SearchPattern::compile(r"\N{GRINNING FACE}", SearchLimits::default()).unwrap();
    assert_eq!(
        pattern
            .find(&document.layout, &Cancellation::default())
            .unwrap()
            .hits
            .len(),
        1
    );
}

#[test]
fn character_ranges_can_start_with_a_literal_hyphen() {
    let document = document("<p>-./01abc</p>", 80);
    // #NOTE: The range starts at hyphen and ends at digit one, including dot, slash, and zero.
    let pattern = SearchPattern::compile("[--1]", SearchLimits::default()).unwrap();
    assert_eq!(
        pattern
            .find(&document.layout, &Cancellation::default())
            .unwrap()
            .hits
            .len(),
        5
    );
}

#[test]
fn unicode_backreferences_compare_simple_lowercase_characters() {
    let document = document("<p>iİ</p>", 80);
    // #NOTE: The backreference compares the captured i with the simple lowercase of dotted capital I.
    let pattern = SearchPattern::compile(r"(i)\1", SearchLimits::default()).unwrap();
    assert_eq!(
        pattern
            .find(&document.layout, &Cancellation::default())
            .unwrap()
            .hits
            .len(),
        1
    );
}

#[test]
fn unicode_backreferences_do_not_expand_long_s_case_equivalence() {
    let document = document("<p>sſ ss</p>", 80);
    // #NOTE: Literal case matching includes long s, but backreferences compare simple lowercase values.
    let pattern = SearchPattern::compile(r"(s)\1", SearchLimits::default()).unwrap();
    assert_eq!(
        pattern
            .find(&document.layout, &Cancellation::default())
            .unwrap()
            .hits
            .len(),
        1
    );
}

#[test]
fn ascii_backreferences_do_not_fold_nonascii_characters() {
    let document = document("<p>Àà</p>", 80);
    // #NOTE: ASCII-scoped backreferences preserve non-ASCII characters exactly.
    let pattern = SearchPattern::compile(r"(?a:([Àà])\1)", SearchLimits::default()).unwrap();
    assert!(
        pattern
            .find(&document.layout, &Cancellation::default())
            .unwrap()
            .hits
            .is_empty()
    );
}

#[test]
fn backreference_case_mapping_does_not_change_with_the_rust_toolchain() {
    let document = document("<p>\u{1c89}\u{1c8a}</p>", 80);
    // #NOTE: This pair gained a case mapping after Unicode 15.0; established backreferences leave both scalars unchanged.
    let pattern = SearchPattern::compile(r"(\u1C89)\1", SearchLimits::default()).unwrap();
    assert!(
        pattern
            .find(&document.layout, &Cancellation::default())
            .unwrap()
            .hits
            .is_empty()
    );
}

#[test]
fn global_search_flags_must_precede_pattern_atoms() {
    // #NOTE: Global case flags belong at the start; scoped flags can appear inside the pattern.
    assert!(SearchPattern::compile("a(?i)b", SearchLimits::default()).is_err());
}

#[test]
fn invalid_search_patterns_report_errors() {
    assert!(SearchPattern::compile("[", SearchLimits::default()).is_err());
}

#[test]
fn canceled_searches_do_not_return_partial_results() {
    let document = document("<p>alpha beta</p>", 80);
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(matches!(
        SearchPattern::compile("alpha", SearchLimits::default())
            .unwrap()
            .find(&document.layout, &cancellation),
        Err(ers::Error::Canceled)
    ));
}

#[test]
fn active_backtracking_stops_when_the_search_is_canceled() {
    let document = document(&format!("<pre>{}!</pre>", "a".repeat(200)), 80);
    let cancellation = Cancellation::default();
    let worker_cancel = cancellation.clone();
    let (started, ready) = crossbeam_channel::bounded(1);
    let (sender, result) = crossbeam_channel::bounded(1);
    let worker = std::thread::spawn(move || {
        let limits = SearchLimits {
            match_steps_max: u32::MAX,
            depth_max: 100_000,
            ..SearchLimits::default()
        };
        // #NOTE: Ambiguous alternatives keep the engine backtracking while another thread cancels it.
        let pattern = SearchPattern::compile(r"(a|aa)+$", limits).unwrap();
        started.send(()).unwrap();
        sender
            .send(pattern.find(&document.layout, &worker_cancel))
            .unwrap();
    });
    ready
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    assert!(
        result.is_empty(),
        "The fixture must still be matching before cancellation"
    );
    cancellation.cancel();
    assert!(matches!(
        result
            .recv_timeout(std::time::Duration::from_millis(500))
            .unwrap(),
        Err(ers::Error::Canceled)
    ));
    worker.join().unwrap();
}

#[test]
fn search_deadlines_report_a_clear_time_limit_error() {
    let document = document(&format!("<pre>{}!</pre>", "a".repeat(200)), 80);
    let limits = SearchLimits {
        match_steps_max: u32::MAX,
        depth_max: 100_000,
        duration_max: std::time::Duration::from_millis(10),
        ..SearchLimits::default()
    };
    // #NOTE: Ambiguous alternatives force long backtracking unless the deadline interrupts execution.
    let pattern = SearchPattern::compile(r"(a|aa)+$", limits).unwrap();
    let error = pattern
        .find(&document.layout, &Cancellation::default())
        .unwrap_err();
    assert!(error.to_string().contains("time limit"), "{error}");
}

#[test]
fn pathological_search_patterns_hit_a_bounded_match_limit() {
    let document = document(&format!("<pre>{}!</pre>", "a".repeat(500)), 80);
    let limits = SearchLimits {
        match_steps_max: 1000,
        ..SearchLimits::default()
    };
    // #NOTE: Nested repetitions force excessive backtracking before a required end-of-line match.
    let pattern = SearchPattern::compile(r"(a+)+$", limits).unwrap();
    assert!(
        pattern
            .find(&document.layout, &Cancellation::default())
            .is_err()
    );
}
