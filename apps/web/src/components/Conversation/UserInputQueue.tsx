import { useEffect, useMemo, useState } from "react";
import Check from "lucide-react/dist/esm/icons/check";
import CircleHelp from "lucide-react/dist/esm/icons/circle-help";
import type {
  PendingUserInputSummary,
  UserInputQuestionSummary,
} from "../../../browser/types";

type Answers = Record<string, { answers: string[] }>;

type UserInputQueueProps = {
  requests: PendingUserInputSummary[];
  submittingIds: Set<string>;
  onSubmit: (requestId: string, version: number, answers: Answers) => Promise<void> | void;
};

type QuestionState = {
  selectedIndex: number | null;
  custom: boolean;
  text: string;
};

function initialQuestionState(): QuestionState {
  return { selectedIndex: null, custom: false, text: "" };
}

function sourceLabel(request: PendingUserInputSummary): string {
  return request.source.kind === "agent"
    ? `${request.source.displayTitle} needs your input`
    : "Your input is needed";
}

function questionIsComplete(question: UserInputQuestionSummary, state: QuestionState | undefined) {
  if (!state) return false;
  if (state.selectedIndex !== null) return true;
  if (question.options.length > 0 && !state.custom) return false;
  return state.text.trim().length > 0;
}

function answerForQuestion(
  question: UserInputQuestionSummary,
  state: QuestionState | undefined,
) {
  if (!state) return [];
  if (state.selectedIndex !== null) {
    const option = question.options[state.selectedIndex];
    return option?.label?.trim() ? [option.label.trim()] : [];
  }
  const value = state.text.trim();
  return value ? [value] : [];
}

function PendingUserInputCard({
  request,
  submitting,
  onSubmit,
}: {
  request: PendingUserInputSummary;
  submitting: boolean;
  onSubmit: UserInputQueueProps["onSubmit"];
}) {
  const [questionStates, setQuestionStates] = useState<Record<string, QuestionState>>({});

  useEffect(() => {
    setQuestionStates(Object.fromEntries(
      request.questions.map((question) => [question.id, initialQuestionState()]),
    ));
  }, [request.id, request.questions]);

  const complete = useMemo(
    () => request.questions.every((question) =>
      questionIsComplete(question, questionStates[question.id])),
    [questionStates, request.questions],
  );

  const selectOption = (questionId: string, index: number) => {
    setQuestionStates((current) => ({
      ...current,
      [questionId]: { selectedIndex: index, custom: false, text: "" },
    }));
  };

  const startCustomInput = (questionId: string) => {
    setQuestionStates((current) => ({
      ...current,
      [questionId]: { selectedIndex: null, custom: true, text: current[questionId]?.text ?? "" },
    }));
  };

  const setText = (questionId: string, text: string) => {
    setQuestionStates((current) => ({
      ...current,
      [questionId]: { selectedIndex: null, custom: true, text },
    }));
  };

  const submit = () => {
    const answers: Answers = {};
    for (const question of request.questions) {
      answers[question.id] = {
        answers: answerForQuestion(question, questionStates[question.id]),
      };
    }
    void onSubmit(request.id, request.version, answers);
  };

  return (
    <div className="web-user-input-card" role="group" aria-label={sourceLabel(request)}>
      <div className="web-user-input-title">
        <CircleHelp size={16} aria-hidden="true" />
        <span>{sourceLabel(request)}</span>
      </div>
      {request.questions.map((question) => {
        const state = questionStates[question.id] ?? initialQuestionState();
        return (
          <section className="web-user-input-question" key={question.id}>
            {question.header ? <div className="web-user-input-header">{question.header}</div> : null}
            <div className="web-user-input-prompt">{question.question}</div>
            {question.options.length ? (
              <div className="web-user-input-options">
                {question.options.map((option, index) => {
                  const selected = state.selectedIndex === index;
                  return (
                    <button
                      type="button"
                      className={`web-user-input-option${selected ? " is-selected" : ""}`}
                      key={`${question.id}-${option.label}-${index}`}
                      disabled={submitting}
                      onClick={() => selectOption(question.id, index)}
                    >
                      <span className="web-user-input-check">
                        {selected ? <Check size={13} aria-hidden="true" /> : null}
                      </span>
                      <span>
                        <strong>{option.label}</strong>
                        {option.description ? <small>{option.description}</small> : null}
                      </span>
                    </button>
                  );
                })}
                {question.isOther ? (
                  <button
                    type="button"
                    className={`web-user-input-option${state.custom ? " is-selected" : ""}`}
                    disabled={submitting}
                    onClick={() => startCustomInput(question.id)}
                  >
                    <span className="web-user-input-check">{state.custom ? "✓" : null}</span>
                    <span><strong>Custom input</strong></span>
                  </button>
                ) : null}
              </div>
            ) : null}
            {(!question.options.length || (question.isOther && state.custom)) ? (
              question.isSecret ? (
                <input
                  type="password"
                  className="web-user-input-note"
                  value={state.text}
                  disabled={submitting}
                  onChange={(event) => setText(question.id, event.target.value)}
                  placeholder="Type your answer"
                />
              ) : (
                <textarea
                  className="web-user-input-note"
                  value={state.text}
                  disabled={submitting}
                  onChange={(event) => setText(question.id, event.target.value)}
                  placeholder="Type your answer"
                  rows={2}
                />
              )
            ) : null}
          </section>
        );
      })}
      <div className="web-user-input-actions">
        <button type="button" disabled={!complete || submitting} onClick={submit}>
          {submitting ? "Submitting…" : "Submit"}
        </button>
      </div>
    </div>
  );
}

export default function UserInputQueue({ requests, submittingIds, onSubmit }: UserInputQueueProps) {
  if (requests.length === 0) return null;
  return (
    <div className="web-user-input-queue" aria-label="Pending user inputs">
      {requests.map((request) => (
        <PendingUserInputCard
          key={request.id}
          request={request}
          submitting={submittingIds.has(request.id)}
          onSubmit={onSubmit}
        />
      ))}
    </div>
  );
}
