# Timelock Operator Procedure

This document specifies how pending timelock actions are reviewed, cancelled,
expired, and communicated before execution. It applies to every action queued
through the governance timelock (parameter changes, upgrades, admin transfers,
and any other privileged operation).

## 1. Reviewing pending actions before execution

**Who reviews.** Every queued action requires review by at least two
independent operators (the *reviewers*) who did not author the proposal. The
multisig signers who will approve execution act as the final gate.

**What is checked.** For each pending action, reviewers confirm:

- The action's target contract, function, and arguments match the approved
  proposal (no unexpected calldata).
- The action is within the documented scope of the proposal and does not touch
  unrelated state.
- The execution point (ETA) leaves enough time for stakeholder notice (see §4).
- Preconditions still hold (e.g. referenced addresses, thresholds, and roles are
  current).
- A rehearsal or dry-run has been performed where the tooling supports it.

**Approval and communication steps.**

1. Author queues the action and records the proposal ID, ETA, and calldata hash.
2. Two reviewers independently verify the items above and record their sign-off
   (proposal ID + calldata hash) in the governance channel.
3. Reviewers post the stakeholder notice described in §4.
4. Only after sign-off and notice does the action become eligible for execution
   at or after its ETA.

If any check fails, the action is cancelled per §2 rather than executed.

## 2. Cancelling pending actions before execution

A pending action can be cancelled at any time before its execution point.

**Authorization.** Cancellation requires the same authority that can execute the
action (the governance multisig / timelock admin). Any single reviewer may
*request* cancellation; the multisig must authorize it on-chain.

**State transitions.**

- `Pending` → `Cancelled`: the action is removed from the executable set and can
  no longer be executed, even after its ETA.
- `Pending` → `Executed`: only permitted at or after the ETA and only if the
  action was not cancelled first.
- `Cancelled` is terminal; a cancelled action cannot be revived. A replacement
  must be queued as a new proposal with a fresh review and notice cycle.

**Rehearsal requirement.** Before relying on cancellation in production, run a
rehearsal on a test network: queue a deliberately unsafe operation, confirm it
is `Pending`, cancel it, and verify that an execution attempt after the ETA
fails. Record the rehearsal result with the release notes.

## 3. Expiry and boundary conditions

- Each action has a bounded validity window. An action that is not executed
  before its expiry is treated as expired and becomes non-executable.
- Boundary conditions:
  - Execution exactly at the ETA is allowed; execution before the ETA is
    rejected.
  - Execution at or after expiry is rejected; the action must be re-queued.
  - Cancellation takes precedence over both execution and expiry.
- Expiry is surfaced to operators by monitoring the pending-action set and
  alerting when an action is within one review cycle of expiry without sign-off.
  Expired actions are reported in the governance channel with their proposal ID
  and reason (expired vs. cancelled).

## 4. Communicating pending actions to stakeholders

Before execution, operators publish a notice containing:

- Proposal ID, summary, and calldata hash.
- Target contract/function and expected effect.
- ETA and expiry.
- Reviewer sign-offs and any known risks.
- Cancellation status and how to raise an objection.

The notice is posted at queue time and updated on any state change (sign-off,
 cancellation, expiry). Escalation: if an objection is raised or a reviewer
 withdraws sign-off before the ETA, execution is paused and the action is
 cancelled per §2 until a new review cycle completes.

## 5. Repeatable check

Operators verify the procedure by running the governance rehearsal described in
§2 as part of each release. The rehearsal is the repeatable check that an unsafe
queued operation can be cancelled before its execution point. No secrets or
production credentials are used in the rehearsal.
