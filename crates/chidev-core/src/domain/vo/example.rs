//! An acceptance example as values: what it expects, what a program that ended answered, and the verdict. Reading the
//! files of an example and running the program are infra's (`infra::accept`); everything here is what a judgment
//! reads, so it holds no process vocabulary and needs no process to be tested.
//!
//! An example is written in the terms of a program's answer: its standard output, its exit code and, when the example
//! says so, its standard error. A program that gave no answer (it could not start, it did not end in time, a signal
//! ended it) is not described here; the runner words that as [`Why::Abnormal`].

use std::fmt;

use thiserror::Error;

/// Why a value of an example is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExampleError {
    /// A name is empty or holds a character a file name of an example cannot.
    #[error("an example name is not empty and has no `/`, NUL or line feed: {0:?}")]
    BadName(String),
    /// The expected output has no last line `#exit <code>`.
    #[error("does not end with a line #exit <code>")]
    NoExitLine,
}

/// The name of an example: the part of its file names before the extension (`a1` for `a1.expected`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExampleName(String);

impl ExampleName {
    /// The name `name`: not empty, and with no `/`, NUL or line feed, so that it is one file name and one line of a
    /// report.
    pub fn new(name: &str) -> Result<ExampleName, ExampleError> {
        if name.is_empty() || name.contains(['/', '\0', '\n']) {
            return Err(ExampleError::BadName(name.to_string()));
        }
        Ok(ExampleName(name.to_string()))
    }

    /// The name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExampleName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What an example expects of a program: its standard output, its exit code and, when the example has a `.stderr`
/// file, its standard error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expected {
    output: Vec<u8>,
    exit: i32,
    errors: Option<Vec<u8>>,
}

impl Expected {
    /// Reads the text of a `.expected` file: the expected standard output, then a last line `#exit <code>`. One
    /// trailing line feed is not part of that line; the output is everything before it and keeps its own final line
    /// feed. `errors` is the content of the `.stderr` file when there is one; without it standard error is not
    /// compared.
    pub fn parse(expected: &[u8], errors: Option<Vec<u8>>) -> Result<Expected, ExampleError> {
        let text = expected.strip_suffix(b"\n").unwrap_or(expected);
        let start = text
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |at| at + 1);
        let (output, last) = text.split_at(start);
        let exit = last
            .strip_prefix(b"#exit ")
            .and_then(|code| std::str::from_utf8(code).ok())
            // `i32::from_str` also takes a leading `+`; the line is written with an optional `-` only
            .filter(|code| !code.starts_with('+'))
            .and_then(|code| code.parse::<i32>().ok())
            .ok_or(ExampleError::NoExitLine)?;
        Ok(Expected {
            output: output.to_vec(),
            exit,
            errors,
        })
    }

    /// The expected standard output.
    pub fn output(&self) -> &[u8] {
        &self.output
    }

    /// The expected exit code.
    pub fn exit(&self) -> i32 {
        self.exit
    }

    /// The expected standard error, when the example states it.
    pub fn errors(&self) -> Option<&[u8]> {
        self.errors.as_deref()
    }
}

/// The arguments the text of an `.args` file gives: its words, split on white space. There is no quoting, so an
/// argument that holds white space cannot be written.
pub fn args(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

/// What a program that ended with an exit code gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// What it wrote on standard output.
    pub output: Vec<u8>,
    /// What it wrote on standard error.
    pub errors: Vec<u8>,
    /// Its exit code.
    pub exit: i32,
}

/// Where two texts differ. A text is a list of lines, each ending with a line feed, but for the last, which may not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Difference {
    /// The first line, counted from 1, that is not the same; a side with no such line has `None`.
    Line {
        /// The number of the line.
        line: usize,
        /// The line that was expected, without its line feed, as text.
        expected: Option<String>,
        /// The line that was got, without its line feed, as text.
        got: Option<String>,
    },
    /// The lines are the same and the last line ends differently.
    FinalNewline {
        /// The number of the last line.
        line: usize,
        /// Whether the expected text ends with a line feed (the other does not).
        expected: bool,
    },
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shown = |line: &Option<String>| match line {
            Some(text) => format!("{text:?}"),
            None => "end of output".to_string(),
        };
        match self {
            Difference::Line {
                line,
                expected,
                got,
            } => write!(
                f,
                "at line {line}: expected {}, got {}",
                shown(expected),
                shown(got)
            ),
            Difference::FinalNewline { line, expected } => write!(
                f,
                "at line {line}: expected {} final newline",
                if *expected { "a" } else { "no" }
            ),
        }
    }
}

/// The lines of `text` and whether it ends with a line feed.
fn split_lines(text: &[u8]) -> (Vec<&[u8]>, bool) {
    let mut lines: Vec<&[u8]> = text.split(|byte| *byte == b'\n').collect();
    // the piece after the last line feed (or the whole text, if none) is a line only when it is not empty
    let newline = lines.last().is_some_and(|last| last.is_empty());
    if newline {
        lines.pop();
    }
    (lines, newline)
}

/// The most characters of a line a [`Difference`] shows. A reason is one line of a report and the message of a
/// finding, which a reader of an external check caps; a line of output can be megabytes long.
pub const SHOWN_CHARS: usize = 200;

/// `line` as text, its first [`SHOWN_CHARS`] characters, and `...` after them when it is longer.
fn shown_line(line: &[u8]) -> String {
    let text = String::from_utf8_lossy(line);
    match text.char_indices().nth(SHOWN_CHARS) {
        Some((cut, _)) => format!("{}...", &text[..cut]),
        None => text.into_owned(),
    }
}

/// Where `got` differs from `expected`, if it does.
fn difference(expected: &[u8], got: &[u8]) -> Option<Difference> {
    if expected == got {
        return None;
    }
    let (want, want_newline) = split_lines(expected);
    let (have, have_newline) = split_lines(got);
    let shown = |line: Option<&&[u8]>| line.map(|l| shown_line(l));
    for at in 0..want.len().max(have.len()) {
        if want.get(at) != have.get(at) {
            return Some(Difference::Line {
                line: at + 1,
                expected: shown(want.get(at)),
                got: shown(have.get(at)),
            });
        }
    }
    // the same lines, and the bytes differ: only in the last line feed
    debug_assert_ne!(want_newline, have_newline);
    Some(Difference::FinalNewline {
        line: want.len(),
        expected: want_newline,
    })
}

/// One reason an example failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// A file of the example is missing, cannot be read or is malformed; the text names it.
    Unreadable(String),
    /// The program gave no answer; the text says what happened.
    Abnormal(String),
    /// Standard output differs.
    Output(Difference),
    /// The exit code differs.
    Exit {
        /// The exit code that was expected.
        expected: i32,
        /// The exit code the program gave.
        got: i32,
    },
    /// Standard error differs.
    Errors(Difference),
}

impl fmt::Display for Why {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Why::Unreadable(text) | Why::Abnormal(text) => f.write_str(text),
            Why::Output(difference) => write!(f, "standard output differs {difference}"),
            Why::Exit { expected, got } => write!(f, "exit code: expected {expected}, got {got}"),
            Why::Errors(difference) => write!(f, "standard error differs {difference}"),
        }
    }
}

/// What came of running one example.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The program answered as the example expects.
    Ok,
    /// Every reason it did not, in the order they are told.
    Fail(Vec<Why>),
}

impl Verdict {
    /// A failure for the one reason `why`.
    pub fn fail(why: Why) -> Verdict {
        Verdict::Fail(vec![why])
    }

    /// Whether the example passed.
    pub fn is_ok(&self) -> bool {
        matches!(self, Verdict::Ok)
    }
}

impl fmt::Display for Verdict {
    /// `ok`, or the reasons joined by `; `.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Verdict::Ok => f.write_str("ok"),
            Verdict::Fail(reasons) => {
                for (at, why) in reasons.iter().enumerate() {
                    if at > 0 {
                        f.write_str("; ")?;
                    }
                    write!(f, "{why}")?;
                }
                Ok(())
            }
        }
    }
}

/// Whether the answer of a program that ended is what the example expects: standard output, then the exit code, then
/// standard error when the example states it; every difference is a reason.
pub fn judge(expected: &Expected, answer: &Answer) -> Verdict {
    let mut reasons = Vec::new();
    if let Some(difference) = difference(&expected.output, &answer.output) {
        reasons.push(Why::Output(difference));
    }
    if expected.exit != answer.exit {
        reasons.push(Why::Exit {
            expected: expected.exit,
            got: answer.exit,
        });
    }
    if let Some(errors) = &expected.errors
        && let Some(difference) = difference(errors, &answer.errors)
    {
        reasons.push(Why::Errors(difference));
    }
    if reasons.is_empty() {
        Verdict::Ok
    } else {
        Verdict::Fail(reasons)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected(text: &str) -> Expected {
        Expected::parse(text.as_bytes(), None).unwrap()
    }

    fn answer(output: &str, errors: &str, exit: i32) -> Answer {
        Answer {
            output: output.as_bytes().to_vec(),
            errors: errors.as_bytes().to_vec(),
            exit,
        }
    }

    fn line(line: usize, expected: Option<&str>, got: Option<&str>) -> Difference {
        Difference::Line {
            line,
            expected: expected.map(str::to_string),
            got: got.map(str::to_string),
        }
    }

    #[test]
    fn a_name_is_one_file_name_and_one_line() {
        assert_eq!(ExampleName::new("a1").unwrap().as_str(), "a1");
        assert_eq!(ExampleName::new("a.b").unwrap().to_string(), "a.b");
        for bad in ["", "a/b", "a\0b", "a\nb"] {
            assert_eq!(
                ExampleName::new(bad),
                Err(ExampleError::BadName(bad.to_string()))
            );
        }
        assert!(ExampleName::new("a1").unwrap() < ExampleName::new("a10").unwrap());
        assert!(ExampleName::new("a10").unwrap() < ExampleName::new("a2").unwrap());
    }

    #[test]
    fn the_output_keeps_its_final_newline_and_the_last_line_gives_the_exit_code() {
        let parsed = expected("one\ntwo\n#exit 3\n");
        assert_eq!(parsed.output(), b"one\ntwo\n");
        assert_eq!(parsed.exit(), 3);
        assert_eq!(parsed.errors(), None);
    }

    #[test]
    fn the_line_feed_after_the_exit_line_is_optional() {
        assert_eq!(expected("out\n#exit 0").output(), b"out\n");
        assert_eq!(expected("out\n#exit 0\n").output(), b"out\n");
        // only one line feed is the end of the file; the other is an empty last line, which is not `#exit`
        assert_eq!(
            Expected::parse(b"out\n#exit 0\n\n", None),
            Err(ExampleError::NoExitLine)
        );
    }

    #[test]
    fn the_exit_line_is_a_line_of_its_own_and_the_output_before_it_is_kept_whole() {
        // an output that does not end with a line feed cannot be written: the exit line follows a line feed
        assert_eq!(
            Expected::parse(b"no newline#exit 0", None),
            Err(ExampleError::NoExitLine)
        );
        assert_eq!(expected("x\n\n#exit 0").output(), b"x\n\n");
        assert_eq!(expected("\n#exit 0").output(), b"\n");
    }

    #[test]
    fn the_exit_code_is_a_decimal_i32_with_an_optional_minus() {
        assert_eq!(expected("#exit -1").exit(), -1);
        assert_eq!(expected("#exit 255\n").exit(), 255);
        assert_eq!(expected("#exit 007").exit(), 7);
        assert_eq!(expected("#exit 2147483647").exit(), i32::MAX);
        for bad in [
            "#exit x",
            "#exit",
            "#exit ",
            "#exit +1",
            "#exit 1 ",
            "#exit  1",
            "#exit 2147483648",
            "#exit 1.5",
            " #exit 1",
            "#EXIT 1",
        ] {
            assert_eq!(
                Expected::parse(bad.as_bytes(), None),
                Err(ExampleError::NoExitLine),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_file_with_no_exit_line_is_refused() {
        assert_eq!(
            Expected::parse(b"just output\n", None),
            Err(ExampleError::NoExitLine)
        );
        assert_eq!(Expected::parse(b"", None), Err(ExampleError::NoExitLine));
        assert_eq!(Expected::parse(b"\n", None), Err(ExampleError::NoExitLine));
        assert_eq!(
            Expected::parse(b"#exit 0\nmore\n", None),
            Err(ExampleError::NoExitLine)
        );
    }

    #[test]
    fn only_the_exit_line_is_an_empty_output() {
        let parsed = expected("#exit 0");
        assert_eq!(parsed.output(), b"");
        assert_eq!(parsed.exit(), 0);
    }

    #[test]
    fn the_standard_error_is_kept_with_it() {
        let parsed = Expected::parse(b"#exit 0", Some(b"warn\n".to_vec())).unwrap();
        assert_eq!(parsed.errors(), Some(&b"warn\n"[..]));
    }

    #[test]
    fn arguments_are_split_on_white_space() {
        assert_eq!(args("a  b\tc\nd\n"), ["a", "b", "c", "d"]);
        assert_eq!(args("--flag=1 \u{3000}x"), ["--flag=1", "x"]);
        assert!(args("").is_empty());
        assert!(args(" \n\t ").is_empty());
    }

    #[test]
    fn an_answer_that_is_expected_is_ok() {
        let verdict = judge(&expected("hi\n#exit 0\n"), &answer("hi\n", "ignored", 0));
        assert_eq!(verdict, Verdict::Ok);
        assert!(verdict.is_ok());
        assert_eq!(verdict.to_string(), "ok");
        let verdict = judge(
            &Expected::parse(b"#exit 4", Some(b"e\n".to_vec())).unwrap(),
            &answer("", "e\n", 4),
        );
        assert!(verdict.is_ok());
    }

    #[test]
    fn a_different_line_of_the_output_is_told_with_its_number() {
        let verdict = judge(&expected("a\nb\nc\n#exit 0"), &answer("a\nB\nc\n", "", 0));
        assert_eq!(
            verdict,
            Verdict::fail(Why::Output(line(2, Some("b"), Some("B"))))
        );
        assert_eq!(
            verdict.to_string(),
            r#"standard output differs at line 2: expected "b", got "B""#
        );
    }

    #[test]
    fn lines_are_shown_escaped_and_as_lossy_text() {
        let verdict = judge(
            &Expected::parse(b"tab\there\n#exit 0", None).unwrap(),
            &Answer {
                output: b"bad \xff\n".to_vec(),
                errors: Vec::new(),
                exit: 0,
            },
        );
        assert_eq!(
            verdict.to_string(),
            "standard output differs at line 1: expected \"tab\\there\", got \"bad \u{fffd}\""
        );
    }

    #[test]
    fn a_side_with_no_such_line_is_the_end_of_output() {
        let short = judge(&expected("a\nb\n#exit 0"), &answer("a\n", "", 0));
        assert_eq!(
            short.to_string(),
            r#"standard output differs at line 2: expected "b", got end of output"#
        );
        let long = judge(&expected("a\n#exit 0"), &answer("a\nb\n", "", 0));
        assert_eq!(
            long.to_string(),
            r#"standard output differs at line 2: expected end of output, got "b""#
        );
        let empty = judge(&expected("#exit 0"), &answer("\n", "", 0));
        assert_eq!(
            empty.to_string(),
            r#"standard output differs at line 1: expected end of output, got """#
        );
    }

    #[test]
    fn lines_that_differ_only_in_the_final_newline_say_so() {
        let missing = judge(&expected("a\nb\n#exit 0"), &answer("a\nb", "", 0));
        assert_eq!(
            missing,
            Verdict::fail(Why::Output(Difference::FinalNewline {
                line: 2,
                expected: true
            }))
        );
        assert_eq!(
            missing.to_string(),
            "standard output differs at line 2: expected a final newline"
        );
        // an expected output always ends with a line feed (it is what comes before the `#exit` line), so a text
        // expected without one comes from a `.stderr` file
        let errors_without = Expected::parse(b"#exit 0", Some(b"a\nb".to_vec())).unwrap();
        let extra = judge(&errors_without, &answer("", "a\nb\n", 0));
        assert_eq!(
            extra.to_string(),
            "standard error differs at line 2: expected no final newline"
        );
    }

    #[test]
    fn a_long_line_is_shown_cut() {
        let long = "y".repeat(1_000_000);
        let verdict = judge(
            &expected("x\n#exit 0"),
            &answer(&format!("{long}\n"), "", 0),
        );
        let Verdict::Fail(reasons) = &verdict else {
            panic!("a different line fails")
        };
        let Why::Output(Difference::Line { got: Some(got), .. }) = &reasons[0] else {
            panic!("the line differs")
        };
        assert_eq!(got, &format!("{}...", "y".repeat(SHOWN_CHARS)));
        assert!(verdict.to_string().len() < 2 * SHOWN_CHARS);
        // a line of exactly the cap is shown whole
        let at_cap = "z".repeat(SHOWN_CHARS);
        assert_eq!(shown_line(at_cap.as_bytes()), at_cap);
    }

    #[test]
    fn a_different_exit_code_is_told_with_both() {
        let verdict = judge(&expected("#exit 0"), &answer("", "", 3));
        assert_eq!(
            verdict,
            Verdict::fail(Why::Exit {
                expected: 0,
                got: 3
            })
        );
        assert_eq!(verdict.to_string(), "exit code: expected 0, got 3");
    }

    #[test]
    fn the_standard_error_is_compared_only_when_the_example_has_it() {
        let with = Expected::parse(b"#exit 0", Some(b"one\ntwo\n".to_vec())).unwrap();
        let verdict = judge(&with, &answer("", "one\nTWO\n", 0));
        assert_eq!(
            verdict.to_string(),
            r#"standard error differs at line 2: expected "two", got "TWO""#
        );
        let verdict = judge(&with, &answer("", "one\ntwo", 0));
        assert_eq!(
            verdict.to_string(),
            "standard error differs at line 2: expected a final newline"
        );
        // an empty `.stderr` expects nothing on standard error
        let silent = Expected::parse(b"#exit 0", Some(Vec::new())).unwrap();
        assert!(judge(&silent, &answer("", "", 0)).is_ok());
        assert_eq!(
            judge(&silent, &answer("", "x\n", 0)).to_string(),
            r#"standard error differs at line 1: expected end of output, got "x""#
        );
        // without a `.stderr`, anything goes
        assert!(judge(&expected("#exit 0"), &answer("", "anything", 0)).is_ok());
    }

    #[test]
    fn several_reasons_are_told_in_order() {
        let with = Expected::parse(b"a\n#exit 0", Some(b"e\n".to_vec())).unwrap();
        let verdict = judge(&with, &answer("b\n", "f\n", 2));
        assert_eq!(
            verdict,
            Verdict::Fail(vec![
                Why::Output(line(1, Some("a"), Some("b"))),
                Why::Exit {
                    expected: 0,
                    got: 2
                },
                Why::Errors(line(1, Some("e"), Some("f"))),
            ])
        );
        assert_eq!(
            verdict.to_string(),
            "standard output differs at line 1: expected \"a\", got \"b\"; \
             exit code: expected 0, got 2; \
             standard error differs at line 1: expected \"e\", got \"f\""
        );
    }

    #[test]
    fn the_texts_of_reasons_a_runner_gives_are_shown_as_they_are() {
        assert_eq!(
            Verdict::fail(Why::Unreadable("missing a1.stdin".into())).to_string(),
            "missing a1.stdin"
        );
        assert_eq!(
            Verdict::Fail(vec![
                Why::Abnormal("did not end within 1 s".into()),
                Why::Unreadable("x".into())
            ])
            .to_string(),
            "did not end within 1 s; x"
        );
        assert!(!Verdict::fail(Why::Abnormal("x".into())).is_ok());
    }
}
