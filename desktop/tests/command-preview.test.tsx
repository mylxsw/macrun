// @vitest-environment jsdom
import React from "react";
import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { CommandPreview } from "../src/CommandPreview";

let height: number;
let resize: () => void;
let stylesheet: HTMLStyleElement;
beforeEach(() => {
  height = 190;
  stylesheet = document.createElement("style");
  stylesheet.textContent = ".v4-command-text { line-height: 19px; }";
  document.head.append(stylesheet);
  vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockImplementation(
    () => height,
  );
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(callback: () => void) {
        resize = callback;
      }
      observe() {}
      disconnect() {}
    },
  );
});
afterEach(() => {
  cleanup();
  stylesheet.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

test("long wrapped commands start collapsed, expand by keyboard and retain the full text", async () => {
  const user = userEvent.setup();
  const command = "LONG_VARIABLE=" + "a".repeat(600) + "\nprintf 'done'";
  render(<CommandPreview command={command} />);
  const text = screen.getByLabelText("完整命令");
  const toggle = screen.getByRole("button", { name: "展开命令" });
  expect(toggle.getAttribute("aria-expanded")).toBe("false");
  expect(toggle.getAttribute("aria-controls")).toBe(text.id);
  expect(text.textContent).toBe(command);
  await user.tab();
  await user.keyboard("{Enter}");
  expect(
    screen
      .getByRole("button", { name: "收起命令" })
      .getAttribute("aria-expanded"),
  ).toBe("true");
  expect(text.classList.contains("expanded")).toBe(true);
  await user.keyboard("{Enter}");
  expect(text.classList.contains("expanded")).toBe(false);
  expect(text.textContent).toBe(command);
});

test("short commands need no toggle; resizing recalculates wrapped line overflow", () => {
  height = 57;
  render(<CommandPreview command="echo short" />);
  expect(screen.queryByRole("button")).toBeNull();
  act(() => {
    height = 76;
    resize();
  });
  expect(screen.getByRole("button", { name: "展开命令" })).toBeTruthy();
  act(() => {
    height = 38;
    resize();
  });
  expect(screen.queryByRole("button")).toBeNull();
});

test("live updates preserve expansion but switching tasks starts collapsed", async () => {
  const user = userEvent.setup();
  const { rerender } = render(
    <CommandPreview key="task-a" command="command a" />,
  );
  await user.click(screen.getByRole("button", { name: "展开命令" }));
  rerender(<CommandPreview key="task-a" command="command a" />);
  expect(screen.getByRole("button", { name: "收起命令" })).toBeTruthy();
  rerender(<CommandPreview key="task-b" command="command b" />);
  expect(
    screen
      .getByRole("button", { name: "展开命令" })
      .getAttribute("aria-expanded"),
  ).toBe("false");
});
