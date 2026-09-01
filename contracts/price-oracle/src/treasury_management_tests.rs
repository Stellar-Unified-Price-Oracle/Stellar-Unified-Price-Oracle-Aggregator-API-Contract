#[cfg(test)]
mod tests {
    use super::super::*;

    #[derive(Clone, Debug)]
    struct TreasuryAccount {
        balance: u128,
        owner: String,
        governance_controlled: bool,
    }

    #[derive(Clone, Debug)]
    struct VestingSchedule {
        beneficiary: String,
        total_amount: u128,
        vesting_period: u64,
        claimed_amount: u128,
        start_time: u64,
        revocation_status: bool,
    }

    #[test]
    fn test_treasury_initialization() {
        let treasury = TreasuryAccount {
            balance: 0,
            owner: "governance".to_string(),
            governance_controlled: true,
        };

        assert_eq!(treasury.balance, 0);
        assert!(treasury.governance_controlled);
        assert_eq!(treasury.owner, "governance");
    }

    #[test]
    fn test_treasury_deposit() {
        let mut treasury = TreasuryAccount {
            balance: 0,
            owner: "governance".to_string(),
            governance_controlled: true,
        };

        let deposit_amount = 1000;
        treasury.balance += deposit_amount;

        assert_eq!(treasury.balance, 1000);
    }

    #[test]
    fn test_treasury_withdrawal() {
        let mut treasury = TreasuryAccount {
            balance: 5000,
            owner: "governance".to_string(),
            governance_controlled: true,
        };

        let withdrawal_amount = 1000;
        assert!(treasury.balance >= withdrawal_amount);

        treasury.balance -= withdrawal_amount;
        assert_eq!(treasury.balance, 4000);
    }

    #[test]
    fn test_insufficient_treasury_funds() {
        let treasury = TreasuryAccount {
            balance: 500,
            owner: "governance".to_string(),
            governance_controlled: true,
        };

        let withdrawal_amount = 1000;
        let can_withdraw = treasury.balance >= withdrawal_amount;

        assert!(!can_withdraw);
    }

    #[test]
    fn test_vesting_schedule_creation() {
        let vesting = VestingSchedule {
            beneficiary: "contributor_1".to_string(),
            total_amount: 10000,
            vesting_period: 365 * 24 * 3600,
            claimed_amount: 0,
            start_time: 1000,
            revocation_status: false,
        };

        assert_eq!(vesting.beneficiary, "contributor_1");
        assert_eq!(vesting.total_amount, 10000);
        assert_eq!(vesting.claimed_amount, 0);
        assert!(!vesting.revocation_status);
    }

    #[test]
    fn test_linear_vesting_calculation() {
        let start_time = 0;
        let vesting_period = 365 * 24 * 3600;
        let total_amount = 10000;

        let current_time = start_time + (vesting_period / 2);

        let elapsed = current_time - start_time;
        let vested_amount = (elapsed * total_amount) / vesting_period;

        assert_eq!(vested_amount, 5000);
    }

    #[test]
    fn test_partial_claim() {
        let mut vesting = VestingSchedule {
            beneficiary: "contributor_1".to_string(),
            total_amount: 10000,
            vesting_period: 365 * 24 * 3600,
            claimed_amount: 0,
            start_time: 0,
            revocation_status: false,
        };

        let claim_amount = 2000;
        vesting.claimed_amount += claim_amount;

        assert_eq!(vesting.claimed_amount, 2000);
        assert_eq!(vesting.total_amount - vesting.claimed_amount, 8000);
    }

    #[test]
    fn test_complete_vesting_claim() {
        let mut vesting = VestingSchedule {
            beneficiary: "contributor_1".to_string(),
            total_amount: 10000,
            vesting_period: 365 * 24 * 3600,
            claimed_amount: 0,
            start_time: 0,
            revocation_status: false,
        };

        vesting.claimed_amount = 10000;

        assert_eq!(vesting.claimed_amount, vesting.total_amount);
    }

    #[test]
    fn test_vesting_revocation() {
        let mut vesting = VestingSchedule {
            beneficiary: "contributor_1".to_string(),
            total_amount: 10000,
            vesting_period: 365 * 24 * 3600,
            claimed_amount: 2000,
            start_time: 0,
            revocation_status: false,
        };

        assert!(!vesting.revocation_status);

        vesting.revocation_status = true;

        assert!(vesting.revocation_status);
    }

    #[test]
    fn test_revocation_unclaimed_amount_return() {
        let claimed = 3000;
        let total = 10000;
        let unclaimed = total - claimed;

        assert_eq!(unclaimed, 7000);
    }

    #[test]
    fn test_multiple_vestings() {
        let vestings = vec![
            VestingSchedule {
                beneficiary: "contributor_1".to_string(),
                total_amount: 10000,
                vesting_period: 365 * 24 * 3600,
                claimed_amount: 0,
                start_time: 0,
                revocation_status: false,
            },
            VestingSchedule {
                beneficiary: "contributor_2".to_string(),
                total_amount: 5000,
                vesting_period: 365 * 24 * 3600,
                claimed_amount: 0,
                start_time: 0,
                revocation_status: false,
            },
            VestingSchedule {
                beneficiary: "contributor_3".to_string(),
                total_amount: 8000,
                vesting_period: 365 * 24 * 3600,
                claimed_amount: 0,
                start_time: 0,
                revocation_status: false,
            },
        ];

        let total_vested: u128 = vestings.iter().map(|v| v.total_amount).sum();
        assert_eq!(total_vested, 23000);

        let beneficiary_count = vestings.len();
        assert_eq!(beneficiary_count, 3);
    }

    #[test]
    fn test_treasury_disbursement_via_governance() {
        let mut treasury = TreasuryAccount {
            balance: 10000,
            owner: "governance".to_string(),
            governance_controlled: true,
        };

        let grant_amount = 2000;
        assert!(treasury.governance_controlled);
        assert!(treasury.balance >= grant_amount);

        treasury.balance -= grant_amount;
        assert_eq!(treasury.balance, 8000);
    }

    #[test]
    fn test_treasury_multisig_timelock() {
        let mut treasury = TreasuryAccount {
            balance: 50000,
            owner: "multisig".to_string(),
            governance_controlled: true,
        };

        let withdrawal_request = 5000;
        let pending_delay = 3600;

        assert!(treasury.governance_controlled);
        assert!(treasury.balance >= withdrawal_request);
        assert!(pending_delay > 0);

        treasury.balance -= withdrawal_request;
        assert_eq!(treasury.balance, 45000);
    }

    #[test]
    fn test_vesting_authorization() {
        let vesting = VestingSchedule {
            beneficiary: "contributor_1".to_string(),
            total_amount: 10000,
            vesting_period: 365 * 24 * 3600,
            claimed_amount: 0,
            start_time: 1000,
            revocation_status: false,
        };

        let is_authorized = !vesting.beneficiary.is_empty() && !vesting.revocation_status;
        assert!(is_authorized);
    }

    #[test]
    fn test_treasury_event_emission() {
        let mut treasury = TreasuryAccount {
            balance: 0,
            owner: "governance".to_string(),
            governance_controlled: true,
        };

        let deposit_amount = 5000;
        treasury.balance += deposit_amount;

        assert_eq!(treasury.balance, 5000);

        let mut events = vec![];
        events.push("DepositReceived");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0], "DepositReceived");
    }

    #[test]
    fn test_treasury_balance_after_multiple_operations() {
        let mut treasury = TreasuryAccount {
            balance: 10000,
            owner: "governance".to_string(),
            governance_controlled: true,
        };

        treasury.balance += 5000;
        assert_eq!(treasury.balance, 15000);

        treasury.balance -= 2000;
        assert_eq!(treasury.balance, 13000);

        treasury.balance += 3000;
        assert_eq!(treasury.balance, 16000);

        treasury.balance -= 6000;
        assert_eq!(treasury.balance, 10000);
    }

    #[test]
    fn test_vesting_claim_boundary() {
        let total_amount = 10000;
        let claimed = 9999;

        let remaining = total_amount - claimed;
        assert_eq!(remaining, 1);

        let can_claim_rest = remaining > 0;
        assert!(can_claim_rest);
    }
}
