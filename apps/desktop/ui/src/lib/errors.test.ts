import { beforeAll, describe, expect, it } from "vitest";
import { setLanguage } from "./i18n";
import { errorMessage, translateBackendError } from "./errors";

beforeAll(() => setLanguage("en"));

describe("translateBackendError", () => {
  it("renders a coded error with its arguments in English", () => {
    const message = 'mix-local-error:spawn_failed|["/usr/bin/ollama","denied"]';
    expect(translateBackendError(message)).toBe(
      "Could not start /usr/bin/ollama: denied",
    );
  });

  it("keeps unknown codes visible instead of dropping the message", () => {
    const message = "mix-local-error:future_code|[]";
    expect(translateBackendError(message)).toBe(message);
  });

  it("leaves plain backend messages alone", () => {
    expect(errorMessage("provider HTTP 500")).toBe("provider HTTP 500");
  });

  it("unwraps Error instances before translating", () => {
    expect(errorMessage(new Error("mix-local-error:stop_external|[]"))).toBe(
      "This runtime was started outside the app. Stop it from where you started it.",
    );
  });

  it("keeps the raw message when the argument payload is not valid JSON", () => {
    const message = "mix-local-error:spawn_failed|{not json";
    expect(translateBackendError(message)).toBe(message);
  });

  it("keeps the raw message when the code and arguments are not separated", () => {
    const message = "mix-local-error:spawn_failed";
    expect(translateBackendError(message)).toBe(message);
  });

  it("fills missing arguments with an empty string", () => {
    expect(
      translateBackendError('mix-local-error:spawn_failed|["ollama"]'),
    ).toBe("Could not start ollama: ");
  });

  it("unwraps objects that carry a message string", () => {
    expect(errorMessage({ message: "mix-local-error:stop_external|[]" })).toBe(
      "This runtime was started outside the app. Stop it from where you started it.",
    );
  });

  it("stringifies objects without a usable message and handles empty input", () => {
    expect(errorMessage({ message: 42 })).toBe("[object Object]");
    expect(errorMessage({ message: "" })).toBe("[object Object]");
    expect(errorMessage(null)).toBe("");
    expect(errorMessage(42)).toBe("42");
  });
});
