use std::time::{Duration, Instant};

use pons_ir::{Direction, Document, TypeRef, Violation, validate_value};
use serde_json::json;

fn document() -> Document {
    serde_json::from_value(json!({
        "ir_version": "0.2",
        "info": {"title": "Example", "version": "1"},
        "types": {},
    }))
    .unwrap()
}

fn typed(pattern: &str) -> TypeRef {
    serde_json::from_value(json!({"kind": "string", "pattern": pattern})).unwrap()
}

fn judge(pattern: &str, value: &str) -> Result<bool, String> {
    match validate_value(&document(), &typed(pattern), Direction::Both, &json!(value)) {
        Ok(()) => Ok(true),
        Err(found) => match &found[..] {
            [Violation::Pattern { .. }] => Ok(false),
            [Violation::UnreadablePattern { reason, .. }] => Err(reason.clone()),
            other => panic!("{pattern:?} against {value:?} reported {other:?}"),
        },
    }
}

fn finds(pattern: &str, value: &str) {
    assert!(
        judge(pattern, value)
            .unwrap_or_else(|reason| { panic!("{pattern:?} cannot be read at all: {reason}") }),
        "the published client matches {value:?} with {pattern:?}"
    );
}

fn misses(pattern: &str, value: &str) {
    assert!(
        !judge(pattern, value)
            .unwrap_or_else(|reason| { panic!("{pattern:?} cannot be read at all: {reason}") }),
        "the published client does not match {value:?} with {pattern:?}"
    );
}

fn unreadable(pattern: &str) -> String {
    judge(pattern, "anything").expect_err("the pattern should not be readable")
}

#[test]
fn a_dot_skips_the_line_terminators_and_nothing_else() {
    for (value, matched) in [
        ("\n", false),
        ("\r", false),
        ("\u{2028}", false),
        ("\u{2029}", false),
        ("a", true),
        (" ", true),
        ("é", true),
        ("\u{85}", true),
    ] {
        if matched {
            finds("^.$", value);
        } else {
            misses("^.$", value);
        }
    }
}

#[test]
fn a_dot_takes_one_code_unit_because_no_pattern_is_published_with_the_u_flag() {
    finds("^..$", "\u{1f600}");
    finds("^\u{1f600}$", "\u{1f600}");
    finds("^.{4}$", "\u{1f600}\u{1f600}");
    misses("^.$", "\u{1f600}");
    misses("^\u{1f600}\u{1f600}$", "\u{1f600}");

    misses("^[\u{1f600}]$", "\u{1f600}");
    finds("^[\u{1f600}]{2}$", "\u{1f600}");
    finds("^[a-\u{1f600}]$", "z");

    misses("^[\u{1f64f}]$", "x");
}

#[test]
fn s_names_whitespace_and_the_line_terminators_and_not_char_is_whitespace() {
    for value in [
        "\t", "\n", "\u{b}", "\u{c}", "\r", " ", "\u{a0}", "\u{1680}", "\u{2000}", "\u{200a}",
        "\u{2028}", "\u{2029}", "\u{202f}", "\u{205f}", "\u{3000}", "\u{feff}",
    ] {
        finds("^\\s$", value);
        misses("^\\S$", value);
    }

    misses("^\\s$", "\u{85}");
    finds("^\\S$", "\u{85}");
    finds("^[\\s]$", "\u{feff}");
}

#[test]
fn d_and_w_name_ascii_alone_and_their_complements_name_everything_else() {
    finds("^\\d$", "5");
    misses("^\\d$", "٣");
    misses("^\\d$", "۵");
    finds("^\\D$", "٣");
    misses("^\\D$", "5");
    for value in ["_", "a", "Z", "7"] {
        finds("^\\w$", value);
        misses("^\\W$", value);
    }
    for value in ["é", "٣"] {
        misses("^\\w$", value);
        finds("^\\W$", value);
    }
}

#[test]
fn a_word_boundary_is_a_word_character_on_exactly_one_side_of_it() {
    finds("\\bfoo\\b", "éfoo");
    finds("\\bfoo\\b", "foo bar");
    finds("\\bfoo\\b$", "foo");
    misses("^\\bfoo\\b$", "éfooé");
    misses("\\bfoo\\b", "afoo");
    misses("\\b", "é");
    misses("\\b", "");
    misses("^\\bfoo", "éfoo");
    finds("^a\\B", "ab");
    misses("^a\\B", "a");
    misses("^\\B$", "ab");

    finds("^[\\b]$", "\u{8}");
    misses("^\\b$", "b");
}

#[test]
fn a_brace_that_opens_no_quantifier_is_a_literal() {
    for (pattern, literal) in [
        ("^a{$", "a{"),
        ("^a{}$", "a{}"),
        ("^a{,2}$", "a{,2}"),
        ("^a{1,2,3}$", "a{1,2,3}"),
        ("^a{x}$", "a{x}"),
        ("^a{1$", "a{1"),
    ] {
        finds(pattern, literal);
    }

    finds("^a{?$", "a{");
    finds("^a{?$", "a");
    misses("^a{?$", "a{?");
    finds("^a{??$", "a{");
    finds("^a{{2}$", "a{{");

    finds("^a{2}$", "aa");
    finds("^a{2,}$", "aaa");
    finds("^a{0}$", "");
    misses("^a{1,2}$", "a{1,2}");

    finds("^a{,3}$", "a{,3}");
    misses("^a{,3}$", "aa");
    misses("^a{,3}$", "aaa");
    assert_eq!(
        unreadable("^a{2,1}$"),
        "repetition range 2-1 is inverted",
        "numbers out of order are a syntax error, not a literal"
    );
}

#[test]
fn a_short_or_braced_escape_reads_as_the_letter_itself_without_the_u_flag() {
    finds(r"^\u0061$", "a");
    misses(r"^\u61$", "a");
    finds(r"^\u61$", "u61");
    finds(r"^\u{61}$", &"u".repeat(61));
    misses(r"^\u{61}$", "u{61}");
    finds(r"^\x41$", "A");
    misses(r"^\x41$", "x41");
    finds(r"^\x4$", "x4");
    finds(r"^\q$", "q");
    finds(r"^\-$", "-");
}

#[test]
fn an_escape_the_client_reads_differently_is_refused_rather_than_matched() {
    assert_eq!(unreadable(r"^\cA$"), "escape \\c is not supported");
    assert_eq!(unreadable(r"^\012$"), "octal escapes are not supported");
    assert_eq!(unreadable(r"^[]$"), "empty character class");
    assert_eq!(unreadable(r"^[^]$"), "empty character class");
    assert_eq!(unreadable("^(?=a)b$"), "lookahead is not supported");
    assert_eq!(unreadable("(?<name>a)"), "unsupported group");
    assert_eq!(unreadable(r"^\k$"), "escape \\k is not supported");
}

#[test]
fn the_anchors_hold_the_ends_of_the_value_because_no_m_flag_is_published() {
    finds("^abc$", "abc");
    misses("^abc$", "abcd");
    misses("^abc$", "xabc");
    misses("abc$", "abc\n");
    misses("^abc", "\nabc");
    finds("abc", "xabcx");
    finds("^$", "");
    misses("^.$", "");
    misses("a", "");
    finds("a|b", "xbx");
    misses("^a|b$", "xbx");
    finds("^a|b$", "xb");
    finds("^(a|ab)c$", "abc");
    finds("^(a|)(b|)$", "ab");
    misses("^(a|)(b|)$", "ba");
}

#[test]
fn a_lazy_quantifier_changes_which_match_is_preferred_and_not_whether_one_exists() {
    finds("^a*?b$", "aab");
    finds("^a+?$", "aaa");
    finds("^a{1,4}?$", "aaaa");
    finds("^a*?$", "");
}

#[test]
fn a_repeat_that_can_match_nothing_still_matches() {
    finds("^(a|)+b$", "b");
    finds("^(a*)*$", "aaa");
    finds("^(|a)$", "");
    finds("^(?:)$", "");
    finds("^(a?)*$", "aa");
    finds("^(a*)+$", "aaa");
    finds("^(?:a|b)*c$", "abc");
    finds("^(?:a?){4}$", "aaa");
    finds("^[a-z]*$", "bbb");
    finds("^[^a]*$", "bbb");
    misses("^[^a]*$", "a");
}

#[test]
fn every_branch_of_an_alternation_is_reached_not_only_the_early_ones() {
    finds("^(a|b|c)$", "c");
    finds("^(a|bb|ccc)$", "ccc");
    finds("^(a|bb|ccc)d$", "bbd");
    finds("^(a|b|c)+$", "abc");
    misses("^(a|b|c)$", "d");
    let branches = (0..100)
        .map(|index| format!("o{index}"))
        .collect::<Vec<_>>()
        .join("|");
    let pattern = format!("^({branches})$");
    finds(&pattern, "o0");
    finds(&pattern, "o50");
    finds(&pattern, "o99");
    misses(&pattern, "o100");
}

#[test]
fn a_repeat_beyond_the_limit_is_refused_instead_of_run() {
    assert_eq!(
        unreadable("^a{1000000}$"),
        "a repetition of 1000000 is too many steps to run"
    );
    assert_eq!(
        unreadable("^a{4097}$"),
        "a repetition of 4097 is too many steps to run"
    );
    finds("^a{4096}$", &"a".repeat(4096));
}

fn on_a_small_stack(work: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(work)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn a_long_value_against_a_repeat_needs_no_stack_of_its_own() {
    on_a_small_stack(|| {
        let long = "a".repeat(200_000);
        finds("^[a-z]*$", &long);
        let near_miss = format!("{long}!");
        let started = Instant::now();
        misses("^[a-z]*$", &near_miss);
        let elapsed = started.elapsed();
        eprintln!("a 200k character near miss took {elapsed:?}");
        assert!(
            elapsed < Duration::from_secs(5),
            "a near miss took {elapsed:?}, so the value is being walked from \
             every position instead of once"
        );
    });
}

#[test]
fn a_pattern_that_may_begin_anywhere_walks_the_value_once() {
    on_a_small_stack(|| {
        let long = "a".repeat(200_000);
        let started = Instant::now();
        misses("[a-z]+!", &long);
        let elapsed = started.elapsed();
        eprintln!("a 200k character miss against [a-z]+! took {elapsed:?}");
        assert!(
            elapsed < Duration::from_secs(5),
            "a near miss took {elapsed:?}, so the value is being walked from \
             every position instead of once"
        );
    });
}

#[test]
fn a_long_pattern_needs_no_stack_of_its_own_to_run_against_a_short_value() {
    on_a_small_stack(|| {
        let long = "a".repeat(50_000);
        misses(&format!("^{long}$"), "");
        misses(&format!("^{long}$"), "a");
        misses(&format!("^(a|b){long}$"), "a");
    });
}

#[test]
fn a_long_run_of_optionals_is_walked_in_a_loop() {
    on_a_small_stack(|| {
        let pattern = "^(?:a){0,4000}$";
        finds(pattern, "");
        finds(pattern, &"a".repeat(4000));
        misses(pattern, &"a".repeat(4001));
    });
}

#[test]
fn a_repeat_over_a_long_value_stays_fast_where_a_backtracker_would_not() {
    on_a_small_stack(|| {
        let started = Instant::now();
        misses("^(a*)*b$", &"a".repeat(50_000));
        let elapsed = started.elapsed();
        eprintln!("a 50k character miss against ^(a*)*b$ took {elapsed:?}");
        assert!(elapsed < Duration::from_secs(5), "took {elapsed:?}");
    });
}
