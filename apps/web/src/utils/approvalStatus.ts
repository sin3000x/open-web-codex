export type ApprovalOutcome = "accepted" | "declined" | "answered";

export type ApprovalStatus =
  | "pending"
  | ApprovalOutcome
  | "cancelled"
  | "resolved";

const outcomes = new Set<ApprovalOutcome>([
  "accepted",
  "declined",
  "answered",
]);

export function isApprovalOutcome(
  status: ApprovalStatus | undefined,
): status is ApprovalOutcome {
  return status !== undefined && outcomes.has(status as ApprovalOutcome);
}

export function parseApprovalStatus(value: unknown): ApprovalStatus | undefined {
  return value === "pending"
    || value === "accepted"
    || value === "declined"
    || value === "answered"
    || value === "cancelled"
    || value === "resolved"
    ? value
    : undefined;
}
