//! Exercises the crate through its public API, the way a dependent crate would.

#[test]
fn greets_the_name_it_is_given() {
	assert_eq!(app::greet("ants"), "hello, ants");
}

#[test]
fn an_empty_name_still_produces_a_greeting() {
	assert_eq!(app::greet(""), "hello, ");
}

#[test]
fn a_name_outside_ascii_is_kept_as_is() {
	// Escapes keep the source ASCII: U+8681 is the CJK character for "ant".
	assert_eq!(app::greet("\u{8681}"), "hello, \u{8681}");
}
