import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { api } from "@/lib/api";
import { setLanguage } from "@/lib/i18n";
import { QuestionCard } from "./QuestionCard";
import { createTimeline, type RunTimeline } from "@/lib/runEvents";

beforeAll(() => setLanguage("ja"));

vi.mock("@/lib/api", () => ({
  api: {
    answerQuestion: vi.fn().mockResolvedValue(true),
  },
}));

function live(multiSelect = false): RunTimeline {
  const timeline = createTimeline("run-1");
  timeline.question = {
    questionId: "q-1",
    question: "どれを使いますか?",
    options: [
      { label: "A", description: "first" },
      { label: "B", description: "second" },
    ],
    multiSelect,
    recommended: "A",
  };
  return timeline;
}

describe("QuestionCard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders the question, options and the suggested badge", () => {
    render(<QuestionCard live={live()} />);
    expect(screen.getByText("どれを使いますか?")).toBeInTheDocument();
    expect(screen.getByText("first")).toBeInTheDocument();
    expect(screen.getByText("おすすめ")).toBeInTheDocument();
  });

  it("disables the answer button until something is chosen", () => {
    render(<QuestionCard live={live()} />);
    expect(screen.getByText("回答する").closest("button")).toBeDisabled();
  });

  it("answers with the single selected option", () => {
    render(<QuestionCard live={live()} />);
    const radios = screen.getAllByRole("radio");
    expect(radios).toHaveLength(2);
    fireEvent.click(radios[1]);
    expect(radios[1]).toBeChecked();
    fireEvent.click(screen.getByText("回答する"));
    expect(api.answerQuestion).toHaveBeenCalledWith({
      runId: "run-1",
      questionId: "q-1",
      selected: ["B"],
      freeText: null,
    });
  });

  it("collects several options when multi-select is on", () => {
    render(<QuestionCard live={live(true)} />);
    expect(screen.getByText("複数選択可")).toBeInTheDocument();
    const boxes = screen.getAllByRole("checkbox");
    fireEvent.click(boxes[0]);
    fireEvent.click(boxes[1]);
    fireEvent.click(screen.getByText("回答する"));
    expect(api.answerQuestion).toHaveBeenCalledWith({
      runId: "run-1",
      questionId: "q-1",
      selected: ["A", "B"],
      freeText: null,
    });
  });

  it("sends a free-text note without a selection", () => {
    render(<QuestionCard live={live()} />);
    fireEvent.change(
      screen.getByPlaceholderText("選択肢にない希望があれば書いてください"),
      { target: { value: "おまかせ" } },
    );
    fireEvent.click(screen.getByText("回答する"));
    expect(api.answerQuestion).toHaveBeenCalledWith({
      runId: "run-1",
      questionId: "q-1",
      selected: [],
      freeText: "おまかせ",
    });
  });

  it("renders nothing while the run has no question", () => {
    const { container } = render(
      <QuestionCard live={createTimeline("run-2")} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("lets a multi-select option be unchecked again", () => {
    render(<QuestionCard live={live(true)} />);
    const boxes = screen.getAllByRole("checkbox");
    fireEvent.click(boxes[0]);
    fireEvent.click(boxes[0]);
    fireEvent.change(
      screen.getByPlaceholderText("選択肢にない希望があれば書いてください"),
      { target: { value: "memo" } },
    );
    fireEvent.click(screen.getByText("回答する"));
    expect(api.answerQuestion).toHaveBeenCalledWith({
      runId: "run-1",
      questionId: "q-1",
      selected: [],
      freeText: "memo",
    });
  });

  it("ignores Enter with an empty answer and unrelated keys", () => {
    render(<QuestionCard live={live()} />);
    const note = screen.getByPlaceholderText(
      "選択肢にない希望があれば書いてください",
    );
    fireEvent.keyDown(note, { key: "Enter" });
    fireEvent.change(note, { target: { value: "not sent yet" } });
    fireEvent.keyDown(note, { key: "a" });
    expect(api.answerQuestion).not.toHaveBeenCalled();
  });

  it("submits on Enter in the note field when an answer is possible", () => {
    render(<QuestionCard live={live()} />);
    fireEvent.change(
      screen.getByPlaceholderText("選択肢にない希望があれば書いてください"),
      { target: { value: "Enter で送信" } },
    );
    fireEvent.keyDown(
      screen.getByPlaceholderText("選択肢にない希望があれば書いてください"),
      { key: "Enter" },
    );
    expect(api.answerQuestion).toHaveBeenCalledWith({
      runId: "run-1",
      questionId: "q-1",
      selected: [],
      freeText: "Enter で送信",
    });
  });

  it("keeps the choice visible with a hint when the run already finished", async () => {
    vi.mocked(api.answerQuestion).mockResolvedValueOnce(false);
    render(<QuestionCard live={live()} />);
    fireEvent.click(screen.getAllByRole("radio")[0]);
    fireEvent.click(screen.getByText("回答する"));
    expect(
      await screen.findByText(/実行が終了したため回答は届きませんでした/),
    ).toBeInTheDocument();
    expect(screen.getAllByRole("radio")[0]).toBeChecked();
  });

  it("surfaces a failed answer instead of silently re-enabling the card", async () => {
    vi.mocked(api.answerQuestion).mockRejectedValueOnce(new Error("boom"));
    render(<QuestionCard live={live()} />);
    fireEvent.click(screen.getAllByRole("radio")[0]);
    fireEvent.click(screen.getByText("回答する"));
    await waitFor(() => expect(screen.getByText("boom")).toBeInTheDocument());
  });

  it("falls back to a generic error when the failure carries no message", async () => {
    vi.mocked(api.answerQuestion).mockRejectedValueOnce(null);
    render(<QuestionCard live={live()} />);
    fireEvent.click(screen.getAllByRole("radio")[0]);
    fireEvent.click(screen.getByText("回答する"));
    expect(await screen.findByText("エラー")).toBeInTheDocument();
  });

  it("clears the error on a successful retry", async () => {
    vi.mocked(api.answerQuestion)
      .mockRejectedValueOnce(new Error("boom"))
      .mockResolvedValueOnce(true);
    render(<QuestionCard live={live()} />);
    fireEvent.click(screen.getAllByRole("radio")[0]);
    const submit = screen.getByText("回答する");
    fireEvent.click(submit);
    await waitFor(() => expect(screen.getByText("boom")).toBeInTheDocument());
    fireEvent.click(submit);
    await waitFor(() =>
      expect(screen.queryByText("boom")).not.toBeInTheDocument(),
    );
    expect(api.answerQuestion).toHaveBeenCalledTimes(2);
  });

  it("falls back to the run id and hides empty option descriptions", () => {
    const timeline = createTimeline("run-3");
    timeline.question = {
      questionId: null,
      question: "どれ?",
      options: [{ label: "A", description: "" }],
      multiSelect: false,
    };
    render(<QuestionCard live={timeline} />);
    expect(screen.getByRole("group")).toHaveAccessibleName(
      "質問があります: どれ?",
    );
    fireEvent.click(screen.getAllByRole("radio")[0]);
    fireEvent.click(screen.getByText("回答する"));
    expect(api.answerQuestion).toHaveBeenCalledWith({
      runId: "run-3",
      questionId: null,
      selected: ["A"],
      freeText: null,
    });
    expect(
      vi.mocked(api.answerQuestion).mock.calls.at(-1)?.[0].questionId,
    ).toBeNull();
  });
});
