use anchor_lang::prelude::*;

use crate::constants::{CLOSE_SECONDS, OPEN_SECONDS, SECONDS_PER_DAY};

/// Mirrors `mpl_core::types::ExternalValidationResult` (same Borsh layout).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum OracleResult {
    Approved,
    Rejected,
    Pass,
}

/// Mirrors `mpl_core::types::OracleValidation` (same Borsh layout). MPL Core
/// reads this straight out of our account data, right after the 8 byte Anchor
/// discriminator (`ValidationResultsOffset::Anchor`).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum OracleValidation {
    Uninitialized,
    V1 {
        create: OracleResult,
        transfer: OracleResult,
        burn: OracleResult,
        update: OracleResult,
    },
}

/// Static (PDA) Oracle account read by MPL Core on every `Transfer` of an
/// asset in the collection.
///
/// NOTE: `validation` MUST stay the first field.
#[account]
#[derive(InitSpace)]
pub struct Oracle {
    pub validation: OracleValidation,
    /// Lamports paid out of the vault to whoever cranks at the right time.
    pub crank_reward: u64,
    /// A crank is only paid if it lands within this many seconds after a
    /// window boundary (open / close).
    pub crank_window: i64,
    pub last_updated: i64,
    pub bump: u8,
    pub vault_bump: u8,
}

impl Oracle {
    pub fn transfer_result(&self) -> Option<OracleResult> {
        match self.validation {
            OracleValidation::V1 { transfer, .. } => Some(transfer),
            OracleValidation::Uninitialized => None,
        }
    }

    /// Validation state for a given transfer verdict. Only `Transfer` is policed;
    /// every other lifecycle event is left to the asset/plugins (`Pass`).
    pub fn validation_for(transfer: OracleResult) -> OracleValidation {
        OracleValidation::V1 {
            create: OracleResult::Pass,
            transfer,
            burn: OracleResult::Pass,
            update: OracleResult::Pass,
        }
    }
}

/// What the `Transfer` verdict must be at `unix_ts`: approved inside
/// [09:00, 17:00) UTC, rejected otherwise.
pub fn window_verdict(unix_ts: i64) -> OracleResult {
    let second_of_day = unix_ts.rem_euclid(SECONDS_PER_DAY);
    if (OPEN_SECONDS..CLOSE_SECONDS).contains(&second_of_day) {
        OracleResult::Approved
    } else {
        OracleResult::Rejected
    }
}

/// Seconds elapsed since the most recent window boundary (open or close).
pub fn seconds_since_boundary(unix_ts: i64) -> i64 {
    let second_of_day = unix_ts.rem_euclid(SECONDS_PER_DAY);
    if second_of_day >= CLOSE_SECONDS {
        second_of_day - CLOSE_SECONDS
    } else if second_of_day >= OPEN_SECONDS {
        second_of_day - OPEN_SECONDS
    } else {
        // before today's open: the last boundary was yesterday's close
        second_of_day + (SECONDS_PER_DAY - CLOSE_SECONDS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = SECONDS_PER_DAY;
    // 2025-01-06 00:00:00 UTC
    const MIDNIGHT: i64 = 1_736_121_600;

    #[test]
    fn verdict_flips_exactly_on_the_boundaries() {
        let at = |h: i64, m: i64, s: i64| MIDNIGHT + h * 3_600 + m * 60 + s;

        assert_eq!(window_verdict(at(0, 0, 0)), OracleResult::Rejected);
        assert_eq!(window_verdict(at(8, 59, 59)), OracleResult::Rejected);
        assert_eq!(window_verdict(at(9, 0, 0)), OracleResult::Approved);
        assert_eq!(window_verdict(at(12, 0, 0)), OracleResult::Approved);
        assert_eq!(window_verdict(at(16, 59, 59)), OracleResult::Approved);
        assert_eq!(window_verdict(at(17, 0, 0)), OracleResult::Rejected);
        assert_eq!(window_verdict(at(23, 59, 59)), OracleResult::Rejected);
    }

    #[test]
    fn verdict_repeats_every_day_and_handles_pre_epoch_timestamps() {
        for day in [-3, -1, 0, 1, 400] {
            let noon = MIDNIGHT + day * DAY + 12 * 3_600;
            assert_eq!(window_verdict(noon), OracleResult::Approved, "day {day}");
            assert_eq!(window_verdict(noon + 6 * 3_600), OracleResult::Rejected);
        }
        // a negative unix timestamp must not panic or wrap
        assert_eq!(window_verdict(-DAY + 10 * 3_600), OracleResult::Approved);
        assert_eq!(window_verdict(-1), OracleResult::Rejected);
    }

    #[test]
    fn seconds_since_boundary_counts_from_the_latest_open_or_close() {
        let at = |h: i64, m: i64, s: i64| MIDNIGHT + h * 3_600 + m * 60 + s;

        assert_eq!(seconds_since_boundary(at(9, 0, 0)), 0);
        assert_eq!(seconds_since_boundary(at(9, 0, 30)), 30);
        assert_eq!(seconds_since_boundary(at(16, 59, 59)), 8 * 3_600 - 1);
        assert_eq!(seconds_since_boundary(at(17, 0, 0)), 0);
        assert_eq!(seconds_since_boundary(at(17, 5, 0)), 300);
        // after midnight the latest boundary is yesterday's 17:00
        assert_eq!(seconds_since_boundary(at(0, 0, 0)), 7 * 3_600);
        assert_eq!(seconds_since_boundary(at(8, 59, 59)), 16 * 3_600 - 1);
    }
}
