//! The strength indicator of the master password (UI 13.1): the zxcvbn
//! estimate (crate `zxcvbn`, MIT) reduced to four levels. It is advice only:
//! the one rule is the minimum length of 8 characters (decision D29).
//!
//! Only the score leaves this module. The zxcvbn result also holds the
//! matched parts of the password, so it is never kept, logged or printed.

use super::strings;

/// zxcvbn looks at most at this many characters (its matching is quadratic;
/// longer passwords are strong anyway).
const MAX_CHARS: usize = 64;

/// Strength levels shown as four bar segments.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Strength {
    /// zxcvbn score 0-1.
    Weak,
    /// Score 2.
    Fair,
    /// Score 3.
    Good,
    /// Score 4.
    Strong,
}

impl Strength {
    /// Estimates `password`; `None` for an empty one.
    pub fn of(password: &str) -> Option<Self> {
        if password.is_empty() {
            return None;
        }
        let end = password
            .char_indices()
            .nth(MAX_CHARS)
            .map_or(password.len(), |(index, _)| index);
        let score = u8::from(zxcvbn::zxcvbn(&password[..end], &[]).score());
        Some(match score {
            0 | 1 => Self::Weak,
            2 => Self::Fair,
            3 => Self::Good,
            _ => Self::Strong,
        })
    }

    /// Filled segments out of four.
    pub fn segments(self) -> usize {
        match self {
            Self::Weak => 1,
            Self::Fair => 2,
            Self::Good => 3,
            Self::Strong => 4,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Weak => strings::STRENGTH_WEAK,
            Self::Fair => strings::STRENGTH_FAIR,
            Self::Good => strings::STRENGTH_GOOD,
            Self::Strong => strings::STRENGTH_STRONG,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_follow_the_estimate() {
        assert_eq!(Strength::of(""), None);
        assert_eq!(Strength::of("password"), Some(Strength::Weak));
        assert_eq!(Strength::of("12345678"), Some(Strength::Weak));
        let long = Strength::of("correct horse battery staple violet").unwrap();
        assert_eq!(long, Strength::Strong);
        assert!(Strength::of("Tr0ub4dor&3x").unwrap() >= Strength::Fair);
        assert_eq!(Strength::Weak.segments(), 1);
        assert_eq!(Strength::Strong.segments(), 4);
        assert_eq!(Strength::Good.label(), "Good");
    }

    #[test]
    fn long_passwords_are_cut_on_a_character_boundary() {
        let long = "ж".repeat(500);
        assert!(Strength::of(&long).is_some());
    }
}
