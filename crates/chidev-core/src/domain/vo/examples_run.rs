//! A run of the examples of a Harness as values: what it stands on, and what became of each example. A Harness of
//! examples holds its examples as Cases in the model; running them is a measure, and this is what the rules read
//! afterwards. The judge of one example is [`crate::domain::vo::example::judge`], so a run and `chidev accept`
//! cannot judge differently; what is kept of its verdict is here.
//!
//! A record keeps what the program answered and not what the example expected. The expectation is the example's own
//! text, which the model holds and a person can read there; copying it into every record would keep a second copy
//! of it to go stale, and would keep text that a correction of the record could not tell from the model's.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;

use super::digest::Digest;
use super::example::{Difference, Verdict, Why};
use super::id::{CaseId, InterfaceId};
use super::measure::{BuiltPart, MeasureError, SettingsDigest, SettingsNow, StampPart};
use super::pin_basis::moved_keys;
use super::revision::Revision;

/// What a run of the examples of a Harness stands on, as the model gives it: the revision of the Harness (its
/// examples included), and the Interfaces it builds, each with the digest of the code pinned for it (none when it
/// has no Pin). Nothing else of the model is on it: the Contracts the examples cite do not change what a program
/// answers, and the Interfaces are on it by their pinned code and not by their revision, for the program is built
/// from that code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExamplesBasis {
    /// The revision of the Harness.
    pub harness: Revision,
    /// The digest of the code pinned for each Interface the Harness builds.
    pub builds: BTreeMap<InterfaceId, Option<Digest>>,
}

/// A run's tie to what it ran: the basis, and the settings (the program, the limits, the layout) it ran under. A run
/// is *current* while the model and the settings still give this stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExamplesStamp {
    basis: ExamplesBasis,
    settings: SettingsDigest,
}

impl ExamplesStamp {
    /// The stamp of a run that stood on `basis` and ran under `settings`.
    pub fn new(basis: ExamplesBasis, settings: SettingsDigest) -> ExamplesStamp {
        ExamplesStamp { basis, settings }
    }

    /// What it stood on.
    pub fn basis(&self) -> &ExamplesBasis {
        &self.basis
    }

    /// The settings digest.
    pub fn settings(&self) -> &SettingsDigest {
        &self.settings
    }

    /// What of this stamp has moved, given the `basis` the model gives now and the `settings` that are now: the
    /// Harness itself (its examples), then each Interface it builds, by id, then the settings. Empty when this is
    /// the stamp of a run that stood on `basis` under those settings. Settings that are not known (`Unconfigured`,
    /// `Unreadable`) are a part that moved, for nothing shows the run was made under them.
    pub fn moved(&self, basis: &ExamplesBasis, settings: &SettingsNow) -> Vec<StampPart> {
        // every field is named, so a field added to the stamp or its basis is not left out of the comparison
        let ExamplesStamp {
            basis: ExamplesBasis { harness, builds },
            settings: was_settings,
        } = self;
        let mut parts = Vec::new();
        if *harness != basis.harness {
            parts.push(StampPart::Examples);
        }
        for id in moved_keys(builds, &basis.builds) {
            // the Harness began to build the Interface or stopped, or the code pinned for it is another (a Pin made
            // again, or a first one, or a lost one)
            let part = match (builds.get(&id), basis.builds.get(&id)) {
                (Some(_), Some(_)) => BuiltPart::PinnedCode,
                _ => BuiltPart::Membership,
            };
            parts.push(StampPart::Built(id, part));
        }
        parts.extend(settings.moved_from(was_settings));
        parts
    }

    /// One digest of the whole stamp, for a person to compare two stamps by eye.
    pub fn fingerprint(&self) -> Digest {
        let b = &self.basis;
        let mut text = format!("examples;harness={}", b.harness.number());
        let _ = write!(text, ";builds=");
        for (id, pin) in &b.builds {
            let pin = pin.map_or_else(|| "-".to_string(), |d| d.to_string());
            let _ = write!(text, "{id}:{pin},");
        }
        let _ = write!(text, ";settings={}", self.settings);
        Digest::of(text.as_bytes())
    }
}

/// Where a program's answer differs from what an example expects, as far as the answer tells: the line, and what
/// the program gave there. What the example expected is in the example.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Got {
    /// The first line, counted from 1, that is not the same.
    Line {
        /// The number of the line.
        line: usize,
        /// The line the program gave, without its line feed, as text; none when its output ended before it.
        got: Option<String>,
    },
    /// The lines are the same and the last line ends differently.
    FinalNewline {
        /// The number of the last line.
        line: usize,
        /// Whether the program's output ends with a line feed.
        got_newline: bool,
    },
}

impl Got {
    /// What the program gave, from the difference the judge found. The expected side is left out.
    fn of(difference: &Difference) -> Got {
        match difference {
            Difference::Line { line, got, .. } => Got::Line {
                line: *line,
                got: got.clone(),
            },
            Difference::FinalNewline { line, expected } => Got::FinalNewline {
                line: *line,
                got_newline: !*expected,
            },
        }
    }
}

impl fmt::Display for Got {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Got::Line { line, got } => match got {
                Some(text) => write!(f, "at line {line}: got {text:?}"),
                None => write!(f, "at line {line}: got end of output"),
            },
            Got::FinalNewline { line, got_newline } => write!(
                f,
                "at line {line}: got {} final newline",
                if *got_newline { "a" } else { "no" }
            ),
        }
    }
}

/// One reason an example failed, as a record keeps it: [`Why`] without the expected side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// A file or a text of the example is missing, cannot be read or is malformed.
    Unreadable(String),
    /// The program gave no answer; the text says what happened.
    Abnormal(String),
    /// Standard output differs.
    Output(Got),
    /// Standard error differs.
    Errors(Got),
    /// The exit code differs.
    Exit {
        /// The exit code the program gave.
        got: i32,
    },
}

impl Reason {
    /// The reason a [`Why`] is, with what the example expected left out.
    fn of(why: &Why) -> Reason {
        match why {
            Why::Unreadable(text) => Reason::Unreadable(text.clone()),
            Why::Abnormal(text) => Reason::Abnormal(text.clone()),
            Why::Output(difference) => Reason::Output(Got::of(difference)),
            Why::Errors(difference) => Reason::Errors(Got::of(difference)),
            Why::Exit { got, .. } => Reason::Exit { got: *got },
        }
    }

    /// The text the reason holds that a program or a person wrote: what a correction can replace. The reasons that
    /// are numbers and positions hold none.
    pub fn text(&self) -> Option<&str> {
        match self {
            Reason::Unreadable(text) | Reason::Abnormal(text) => Some(text),
            Reason::Output(Got::Line { got, .. }) | Reason::Errors(Got::Line { got, .. }) => {
                got.as_deref()
            }
            Reason::Output(Got::FinalNewline { .. })
            | Reason::Errors(Got::FinalNewline { .. })
            | Reason::Exit { .. } => None,
        }
    }

    /// The reason with its text replaced by `new`; none for a reason that holds no text.
    pub(crate) fn with_text(&self, new: String) -> Option<Reason> {
        match self {
            Reason::Unreadable(_) => Some(Reason::Unreadable(new)),
            Reason::Abnormal(_) => Some(Reason::Abnormal(new)),
            Reason::Output(Got::Line { line, got: Some(_) }) => Some(Reason::Output(Got::Line {
                line: *line,
                got: Some(new),
            })),
            Reason::Errors(Got::Line { line, got: Some(_) }) => Some(Reason::Errors(Got::Line {
                line: *line,
                got: Some(new),
            })),
            _ => None,
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::Unreadable(text) | Reason::Abnormal(text) => f.write_str(text),
            Reason::Output(got) => write!(f, "standard output differs {got}"),
            Reason::Errors(got) => write!(f, "standard error differs {got}"),
            Reason::Exit { got } => write!(f, "exit code: got {got}"),
        }
    }
}

/// What became of one example.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExampleResult {
    /// The program answered as the example expects.
    Passed,
    /// Every reason it did not, in the order the judge told them.
    Failed(Vec<Reason>),
}

/// One example of a run: the Case it is, by id, and what became of it. The id is the Case's, resolved from the
/// example's name when the run was recorded, so that a Case that is renamed afterwards is still the one (a new
/// name is a move of the record).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExampleRecord {
    /// The Case that holds the example.
    pub case: CaseId,
    /// What became of it.
    pub result: ExampleResult,
}

impl ExampleRecord {
    /// The record of the example of `case` that the judge gave `verdict` for.
    pub fn from_verdict(case: CaseId, verdict: &Verdict) -> ExampleRecord {
        let result = match verdict {
            Verdict::Ok => ExampleResult::Passed,
            Verdict::Fail(why) => ExampleResult::Failed(why.iter().map(Reason::of).collect()),
        };
        ExampleRecord { case, result }
    }

    /// Whether the example passed.
    pub fn passed(&self) -> bool {
        self.result == ExampleResult::Passed
    }
}

/// What a run of examples gave: a record for each example that was run, in the order they were run, and none for a
/// Case twice.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExamplesOutcome(Vec<ExampleRecord>);

impl ExamplesOutcome {
    /// The outcome of these `records`. A Case recorded twice is refused: a run runs each example once, and the later
    /// record would hide the earlier.
    pub fn new(records: Vec<ExampleRecord>) -> Result<ExamplesOutcome, MeasureError> {
        let mut seen = BTreeSet::new();
        for r in &records {
            if !seen.insert(r.case) {
                return Err(MeasureError::DuplicateExample(r.case.to_string()));
            }
        }
        Ok(ExamplesOutcome(records))
    }

    /// The records, in the order the examples were run.
    pub fn records(&self) -> &[ExampleRecord] {
        &self.0
    }

    /// The records, to change what they hold. A slice, so that none is added and a Case cannot come twice.
    pub(crate) fn records_mut(&mut self) -> &mut [ExampleRecord] {
        &mut self.0
    }

    /// How many examples passed.
    pub fn passed(&self) -> usize {
        self.0.iter().filter(|r| r.passed()).count()
    }

    /// The examples that failed, with their reasons, in the order they were run.
    pub fn failed(&self) -> impl Iterator<Item = (CaseId, &[Reason])> {
        self.0.iter().filter_map(|r| match &r.result {
            ExampleResult::Passed => None,
            ExampleResult::Failed(reasons) => Some((r.case, reasons.as_slice())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::vo::example::{Answer, Expected, judge};
    use crate::domain::vo::id::test_uuid;

    fn case(n: u128) -> CaseId {
        CaseId::from_uuid(test_uuid(n))
    }

    fn interface(n: u128) -> InterfaceId {
        InterfaceId::from_uuid(test_uuid(n))
    }

    fn settings(word: &str) -> SettingsDigest {
        SettingsDigest::new(Digest::of(word.as_bytes()))
    }

    fn known(word: &str) -> SettingsNow {
        SettingsNow::Known(settings(word))
    }

    #[test]
    fn a_record_keeps_what_the_program_gave_and_not_what_the_example_expected() {
        let expected = Expected::parse(b"one\ntwo\n#exit 0\n", Some(b"quiet\n".to_vec())).unwrap();
        let answer = |output: &str, errors: &str, exit: i32| Answer {
            output: output.as_bytes().to_vec(),
            errors: errors.as_bytes().to_vec(),
            exit,
        };
        let record = |a: Answer| ExampleRecord::from_verdict(case(1), &judge(&expected, &a));
        assert_eq!(
            record(answer("one\ntwo\n", "quiet\n", 0)).result,
            ExampleResult::Passed
        );
        // a line that differs, the exit code and the errors, in the order the judge tells them
        assert_eq!(
            record(answer("one\n2\n", "loud\n", 3)).result,
            ExampleResult::Failed(vec![
                Reason::Output(Got::Line {
                    line: 2,
                    got: Some("2".to_string())
                }),
                Reason::Exit { got: 3 },
                Reason::Errors(Got::Line {
                    line: 1,
                    got: Some("loud".to_string())
                }),
            ])
        );
        // output that ended early has no line to show
        assert_eq!(
            record(answer("one\n", "quiet\n", 0)).result,
            ExampleResult::Failed(vec![Reason::Output(Got::Line { line: 2, got: None })])
        );
        // the final line feed is told by what the program did, which is the opposite of what was expected
        assert_eq!(
            record(answer("one\ntwo", "quiet\n", 0)).result,
            ExampleResult::Failed(vec![Reason::Output(Got::FinalNewline {
                line: 2,
                got_newline: false
            })])
        );
        // a verdict with no answer behind it
        let unreadable = Verdict::fail(Why::Unreadable("missing a1.args".to_string()));
        let abnormal = Verdict::fail(Why::Abnormal("did not end within 10 s".to_string()));
        assert_eq!(
            ExampleRecord::from_verdict(case(2), &unreadable).result,
            ExampleResult::Failed(vec![Reason::Unreadable("missing a1.args".to_string())])
        );
        assert_eq!(
            ExampleRecord::from_verdict(case(3), &abnormal).result,
            ExampleResult::Failed(vec![Reason::Abnormal(
                "did not end within 10 s".to_string()
            )])
        );
        assert!(ExampleRecord::from_verdict(case(4), &Verdict::Ok).passed());
        assert!(!ExampleRecord::from_verdict(case(2), &unreadable).passed());
    }

    #[test]
    fn a_final_line_feed_the_program_gave_and_the_example_did_not_expect_is_a_got_newline() {
        let expected = Expected::parse(b"one\n#exit 0\n", None).unwrap();
        let output = |text: &str| {
            judge(
                &expected,
                &Answer {
                    output: text.as_bytes().to_vec(),
                    errors: Vec::new(),
                    exit: 0,
                },
            )
        };
        // the expected output ends with a line feed; the program's does not
        assert_eq!(
            ExampleRecord::from_verdict(case(1), &output("one")).result,
            ExampleResult::Failed(vec![Reason::Output(Got::FinalNewline {
                line: 1,
                got_newline: false
            })])
        );
    }

    #[test]
    fn a_reason_is_worded_without_the_expected_side() {
        let said = |r: Reason| r.to_string();
        assert_eq!(
            said(Reason::Output(Got::Line {
                line: 2,
                got: Some("2".to_string())
            })),
            "standard output differs at line 2: got \"2\""
        );
        assert_eq!(
            said(Reason::Errors(Got::Line { line: 1, got: None })),
            "standard error differs at line 1: got end of output"
        );
        assert_eq!(
            said(Reason::Output(Got::FinalNewline {
                line: 3,
                got_newline: true
            })),
            "standard output differs at line 3: got a final newline"
        );
        assert_eq!(said(Reason::Exit { got: 2 }), "exit code: got 2");
        assert_eq!(
            said(Reason::Abnormal("ended by a signal".into())),
            "ended by a signal"
        );
    }

    #[test]
    fn only_the_reasons_that_carry_text_have_it_and_can_have_it_replaced() {
        let line = |got: Option<&str>| Got::Line {
            line: 1,
            got: got.map(str::to_string),
        };
        for (reason, text) in [
            (Reason::Unreadable("u".into()), Some("u")),
            (Reason::Abnormal("a".into()), Some("a")),
            (Reason::Output(line(Some("o"))), Some("o")),
            (Reason::Errors(line(Some("e"))), Some("e")),
            (Reason::Output(line(None)), None),
            (Reason::Errors(line(None)), None),
            (
                Reason::Output(Got::FinalNewline {
                    line: 1,
                    got_newline: true,
                }),
                None,
            ),
            (Reason::Exit { got: 1 }, None),
        ] {
            assert_eq!(reason.text(), text, "{reason:?}");
            let replaced = reason.with_text("new".to_string());
            assert_eq!(replaced.is_some(), text.is_some(), "{reason:?}");
            if let Some(replaced) = replaced {
                assert_eq!(replaced.text(), Some("new"));
            }
        }
    }

    #[test]
    fn an_outcome_refuses_a_case_twice_and_counts_what_passed() {
        let passed = |n| ExampleRecord {
            case: case(n),
            result: ExampleResult::Passed,
        };
        let failed = |n| ExampleRecord {
            case: case(n),
            result: ExampleResult::Failed(vec![Reason::Exit { got: 1 }]),
        };
        assert_eq!(
            ExamplesOutcome::new(vec![passed(1), failed(1)]),
            Err(MeasureError::DuplicateExample(case(1).to_string()))
        );
        let outcome = ExamplesOutcome::new(vec![passed(1), failed(2), passed(3)]).unwrap();
        assert_eq!(outcome.records().len(), 3);
        assert_eq!(outcome.passed(), 2);
        let failures: Vec<_> = outcome.failed().collect();
        assert_eq!(failures, [(case(2), [Reason::Exit { got: 1 }].as_slice())]);
        assert_eq!(ExamplesOutcome::default().records(), []);
    }

    /// A basis of Harness at `harness`, building Interface 4 (code `four`) and Interface 5 (code `five`).
    fn basis(harness: Revision, four: Option<&[u8]>, five: Option<&[u8]>) -> ExamplesBasis {
        ExamplesBasis {
            harness,
            builds: BTreeMap::from([
                (interface(4), four.map(Digest::of)),
                (interface(5), five.map(Digest::of)),
            ]),
        }
    }

    #[test]
    fn a_stamp_says_which_part_moved_and_nothing_when_none_did() {
        let first = Revision::first();
        let stamp = ExamplesStamp::new(basis(first, Some(b"4"), Some(b"5")), settings("s1"));
        let same = basis(first, Some(b"4"), Some(b"5"));
        assert_eq!(stamp.moved(&same, &known("s1")), []);
        // the Harness itself: an example changed
        assert_eq!(
            stamp.moved(&basis(first.next(), Some(b"4"), Some(b"5")), &known("s1")),
            [StampPart::Examples]
        );
        // the code pinned for a built Interface
        assert_eq!(
            stamp.moved(&basis(first, Some(b"4 again"), Some(b"5")), &known("s1")),
            [StampPart::Built(interface(4), BuiltPart::PinnedCode)]
        );
        // an Interface that got a Pin, or lost it, has pinned code that is another
        assert_eq!(
            stamp.moved(&basis(first, Some(b"4"), None), &known("s1")),
            [StampPart::Built(interface(5), BuiltPart::PinnedCode)]
        );
        // an Interface the Harness began to build, and one it stopped building
        let mut more = same.clone();
        more.builds.insert(interface(6), Some(Digest::of(b"6")));
        assert_eq!(
            stamp.moved(&more, &known("s1")),
            [StampPart::Built(interface(6), BuiltPart::Membership)]
        );
        let mut fewer = same.clone();
        fewer.builds.remove(&interface(4));
        assert_eq!(
            stamp.moved(&fewer, &known("s1")),
            [StampPart::Built(interface(4), BuiltPart::Membership)]
        );
    }

    #[test]
    fn settings_that_differ_or_cannot_be_compared_are_a_part_that_moved() {
        let first = Revision::first();
        let stamp = ExamplesStamp::new(basis(first, Some(b"4"), None), settings("s1"));
        let same = basis(first, Some(b"4"), None);
        assert_eq!(
            stamp.moved(&same, &known("s2")),
            [StampPart::Settings(
                crate::domain::vo::measure::SettingsGap::Differ
            )]
        );
        assert_eq!(
            stamp.moved(&same, &SettingsNow::Unconfigured),
            [StampPart::Settings(
                crate::domain::vo::measure::SettingsGap::Unconfigured
            )]
        );
        assert_eq!(
            stamp.moved(&same, &SettingsNow::Unreadable("no layout".to_string())),
            [StampPart::Settings(
                crate::domain::vo::measure::SettingsGap::Unreadable("no layout".to_string())
            )]
        );
    }

    #[test]
    fn several_parts_that_moved_come_in_the_order_of_the_stamp() {
        let first = Revision::first();
        let stamp = ExamplesStamp::new(basis(first, Some(b"4"), Some(b"5")), settings("s1"));
        assert_eq!(
            stamp.moved(&basis(first.next(), Some(b"x"), Some(b"y")), &known("s2")),
            [
                StampPart::Examples,
                StampPart::Built(interface(4), BuiltPart::PinnedCode),
                StampPart::Built(interface(5), BuiltPart::PinnedCode),
                StampPart::Settings(crate::domain::vo::measure::SettingsGap::Differ),
            ]
        );
    }

    #[test]
    fn the_fingerprint_follows_every_part_of_the_stamp() {
        let first = Revision::first();
        let print = |b: ExamplesBasis, s: &str| ExamplesStamp::new(b, settings(s)).fingerprint();
        let base = print(basis(first, Some(b"4"), Some(b"5")), "s");
        assert_eq!(base, print(basis(first, Some(b"4"), Some(b"5")), "s"));
        assert_ne!(
            base,
            print(basis(first.next(), Some(b"4"), Some(b"5")), "s")
        );
        assert_ne!(base, print(basis(first, Some(b"x"), Some(b"5")), "s"));
        assert_ne!(base, print(basis(first, Some(b"4"), None), "s"));
        assert_ne!(base, print(basis(first, Some(b"4"), Some(b"5")), "t"));
    }
}
