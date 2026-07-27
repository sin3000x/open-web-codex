ALTER TABLE approvals
    DROP CONSTRAINT IF EXISTS approvals_decision_check;

ALTER TABLE approvals
    ADD CONSTRAINT approvals_decision_check
    CHECK (decision IS NULL OR decision IN ('approved', 'rejected', 'answered'));

ALTER TABLE approvals
    DROP CONSTRAINT IF EXISTS approvals_state_check;

ALTER TABLE approvals
    ADD CONSTRAINT approvals_state_check
    CHECK (
        state IN (
            'pending',
            'dispatching',
            'delivery_unknown',
            'approved',
            'rejected',
            'answered',
            'expired',
            'cancelled'
        )
    );

UPDATE approvals
SET state = 'answered'
WHERE state = 'approved'
  AND decision = 'answered';
