// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Dates, numbers and ordering in the interface language (roadmap task T24.03).
//!
//! The rest of F24 is text: [`super::vocabulary`] is the identified string set and [`super::catalog`]
//! the pack that fills it. This module is the part of a language that is not a string — how a date is
//! written, how a number is grouped, and how a list is ordered:
//!
//! * **A rule set per language, and nothing guessed.** [`Rules::for_language`] is the only way in;
//!   each rule is a table entry, so a language that writes dates differently is a data change rather
//!   than a branch in a formatter.
//! * **Dates come from the number, not the clock.** [`Stamp::from_epoch`] converts Unix seconds to a
//!   civil UTC date by the proleptic-Gregorian algorithm, so the same commit renders the same way on
//!   every machine. Local time is not applied: no time-zone source is part of this build.
//! * **A claim about a language is only made when the data exists.** Ordering is the case that
//!   matters: English uses an ASCII case fold, Chinese uses code-point order, and the second is
//!   *labelled* [`Collation::CodePoint`] with a note that no collation table ships. Calling code-point
//!   order "Chinese collation" would be a lie about what the list shows.
//! * **Two bundled languages may share a rule.** English and Simplified Chinese group digits the
//!   same way; the table still has an entry per language, so the sameness is a fact about these two
//!   languages rather than a formatter that ignores its language.

use crate::host::locale::Language;

/// A civil date and time in UTC, computed from an epoch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Stamp {
    /// The year (proleptic Gregorian; may be negative).
    pub year: i64,
    /// The month, 1..=12.
    pub month: u32,
    /// The day of the month, 1..=31.
    pub day: u32,
    /// The hour, 0..=23.
    pub hour: u32,
    /// The minute, 0..=59.
    pub minute: u32,
    /// The second, 0..=59.
    pub second: u32,
}

impl Stamp {
    /// The civil UTC date and time of a Unix timestamp.
    ///
    /// The day count is converted with the standard era-based civil-date algorithm; a negative
    /// timestamp is a real input (a commit made before 1970) and is handled by Euclidean division
    /// rather than by a special case.
    pub fn from_epoch(seconds: i64) -> Self {
        let days = seconds.div_euclid(86_400);
        let rem = seconds.rem_euclid(86_400);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let mut year = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        if month <= 2 {
            year += 1;
        }
        Self {
            year,
            month: month as u32,
            day: day as u32,
            hour: (rem / 3_600) as u32,
            minute: ((rem % 3_600) / 60) as u32,
            second: (rem % 60) as u32,
        }
    }
}

/// How a language orders a list of names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Collation {
    /// ASCII letters compared without case, then by code point.
    AsciiFold,
    /// Unicode code-point order, with no language table behind it.
    CodePoint,
}

impl Collation {
    /// The collation's name, as a report writes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::AsciiFold => "ascii-fold",
            Self::CodePoint => "code-point",
        }
    }

    /// What the order is, in a sentence a person can check.
    pub fn note(self) -> &'static str {
        match self {
            Self::AsciiFold => "ASCII letters are compared without case, then by code point",
            Self::CodePoint => {
                "no collation table ships with this build, so the order is code-point order and not \
                 the language's own ordering"
            }
        }
    }
}

/// One language's date, number and ordering rules.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rules {
    language: Language,
}

impl Rules {
    /// The rules the interface language uses.
    pub fn for_language(language: Language) -> Self {
        Self { language }
    }

    /// The date, in the language's written form.
    pub fn date(self, stamp: Stamp) -> String {
        match self.language {
            Language::English => format!("{:04}-{:02}-{:02}", stamp.year, stamp.month, stamp.day),
            Language::SimplifiedChinese => {
                format!("{}年{}月{}日", stamp.year, stamp.month, stamp.day)
            }
        }
    }

    /// The date and time, in the language's written form, followed by `UTC`.
    ///
    /// The time itself is not localised: this build has no time-zone source, and inventing an
    /// offset would make the same commit render differently on two machines.
    pub fn date_time(self, seconds: i64) -> String {
        let stamp = Stamp::from_epoch(seconds);
        format!(
            "{} {:02}:{:02}:{:02} UTC",
            self.date(stamp),
            stamp.hour,
            stamp.minute,
            stamp.second
        )
    }

    /// The digit-grouping separator this language uses.
    pub fn grouping(self) -> char {
        match self.language {
            // Both bundled languages group by three with a comma. The entry exists per language so
            // a language that differs is a table change, not a formatter change.
            Language::English | Language::SimplifiedChinese => ',',
        }
    }

    /// A whole number, grouped in this language's way.
    pub fn number(self, value: i64) -> String {
        let separator = self.grouping();
        let digits = value.unsigned_abs().to_string();
        let mut grouped = String::new();
        for (index, digit) in digits.chars().enumerate() {
            if index > 0 && (digits.len() - index).is_multiple_of(3) {
                grouped.push(separator);
            }
            grouped.push(digit);
        }
        if value < 0 {
            format!("-{grouped}")
        } else {
            grouped
        }
    }

    /// The order this language's lists use.
    pub fn collation(self) -> Collation {
        match self.language {
            Language::English => Collation::AsciiFold,
            Language::SimplifiedChinese => Collation::CodePoint,
        }
    }

    /// The names in this language's order.
    pub fn sorted(self, names: &[String]) -> Vec<String> {
        let collation = self.collation();
        let mut sorted = names.to_vec();
        sorted.sort_by(|left, right| {
            let key = |name: &String| match collation {
                Collation::AsciiFold => name.to_ascii_lowercase(),
                Collation::CodePoint => name.clone(),
            };
            key(left).cmp(&key(right)).then_with(|| left.cmp(right))
        });
        sorted
    }
}

#[cfg(test)]
mod tests {
    use super::{Collation, Rules, Stamp};
    use crate::host::locale::Language;

    fn english() -> Rules {
        Rules::for_language(Language::English)
    }

    fn chinese() -> Rules {
        Rules::for_language(Language::SimplifiedChinese)
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn an_epoch_becomes_a_civil_date_including_before_the_epoch() {
        let stamp = Stamp::from_epoch(1_700_000_000);
        assert_eq!((stamp.year, stamp.month, stamp.day), (2023, 11, 14));
        assert_eq!((stamp.hour, stamp.minute, stamp.second), (22, 13, 20));
        assert_eq!(
            english().date_time(1_700_000_000),
            "2023-11-14 22:13:20 UTC"
        );
        let zero = Stamp::from_epoch(0);
        assert_eq!(
            (zero.year, zero.month, zero.day, zero.hour),
            (1970, 1, 1, 0)
        );
        // A leap day and a second before the epoch are both real inputs.
        let leap = Stamp::from_epoch(1_709_164_800);
        assert_eq!((leap.year, leap.month, leap.day), (2024, 2, 29));
        assert_eq!(english().date_time(-1), "1969-12-31 23:59:59 UTC");
    }

    #[test]
    fn a_date_is_written_the_way_each_language_writes_it() {
        let stamp = Stamp::from_epoch(1_700_000_000);
        assert_eq!(english().date(stamp), "2023-11-14");
        assert_eq!(chinese().date(stamp), "2023年11月14日");
        assert_eq!(
            english().date_time(1_700_000_000),
            "2023-11-14 22:13:20 UTC"
        );
        assert_eq!(
            chinese().date_time(1_700_000_000),
            "2023年11月14日 22:13:20 UTC"
        );
    }

    #[test]
    fn a_number_is_grouped_and_a_negative_sign_is_kept() {
        for rules in [english(), chinese()] {
            assert_eq!(rules.number(0), "0");
            assert_eq!(rules.number(999), "999");
            assert_eq!(rules.number(1_000), "1,000");
            assert_eq!(rules.number(1_234_567), "1,234,567");
            assert_eq!(rules.number(-1_234_567), "-1,234,567");
        }
        // The two bundled languages group the same way; that is a fact about them, not a missing
        // table entry.
        assert_eq!(english().grouping(), chinese().grouping());
    }

    #[test]
    fn ordering_is_labelled_with_the_rule_that_was_used() {
        // English folds case, so `apple` sorts before `Banana`.
        let folded = english();
        assert_eq!(folded.collation(), Collation::AsciiFold);
        assert_eq!(
            folded.sorted(&names(&["Banana", "apple", "Cherry"])),
            names(&["apple", "Banana", "Cherry"])
        );

        // Chinese has no collation table here, and says code-point order rather than claiming one.
        let code_point = chinese();
        assert_eq!(code_point.collation(), Collation::CodePoint);
        assert!(code_point.collation().note().contains("no collation table"));
        assert_eq!(
            code_point.sorted(&names(&["页面", "Button", "apple"])),
            names(&["Button", "apple", "页面"])
        );
    }

    #[test]
    fn an_equal_key_still_has_one_order() {
        // `A` and `a` fold to the same key; the original text breaks the tie, so the answer does not
        // depend on how the sort happened to compare them.
        assert_eq!(english().sorted(&names(&["a", "A"])), names(&["A", "a"]));
    }
}
