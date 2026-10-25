import { describe, expect, it } from "vitest";

import {
  blankCondition,
  blankRules,
  conditionValue,
  draftFromRules,
  OPERATORS,
  rulesFromDraft,
  withField,
} from "./smart-rules";

describe("smart playlist rules", () => {
  it("builds only values the server accepts", () => {
    expect(
      conditionValue({
        field: "year",
        operator: "between",
        value: "1990",
        upper: "1999",
      }),
    ).toEqual([1990, 1999]);
    expect(
      conditionValue({
        field: "year",
        operator: "between",
        value: "2000",
        upper: "1990",
      }),
    ).toBeNull();
    expect(
      conditionValue({
        field: "rating",
        operator: "is",
        value: "6",
        upper: "",
      }),
    ).toBeNull();
    expect(
      conditionValue({
        field: "year",
        operator: "is",
        value: "1999 OR 1",
        upper: "",
      }),
    ).toBeNull();
    expect(
      conditionValue({
        field: "format",
        operator: "is",
        value: ".FLAC",
        upper: "",
      }),
    ).toBe("flac");
    expect(
      conditionValue({
        field: "format",
        operator: "is",
        value: "%",
        upper: "",
      }),
    ).toBeNull();
    expect(
      conditionValue({
        field: "last_played",
        operator: "in_the_last",
        value: "30",
        upper: "",
      }),
    ).toBe(30);
    expect(
      conditionValue({
        field: "date_added",
        operator: "after",
        value: "2024-01-31",
        upper: "",
      }),
    ).toBe("2024-01-31");
    expect(
      conditionValue({
        field: "starred",
        operator: "is",
        value: "true",
        upper: "",
      }),
    ).toBe(true);
    expect(
      conditionValue({
        field: "title",
        operator: "contains",
        value: "   ",
        upper: "",
      }),
    ).toBeNull();
  });

  it("keeps the operator when the new field admits it, and resets it otherwise", () => {
    const draft = {
      ...blankCondition("title"),
      operator: "is" as const,
      value: "x",
    };
    expect(withField(draft, "artist")).toMatchObject({
      operator: "is",
      value: "x",
    });
    expect(withField(draft, "year")).toMatchObject({
      operator: "is",
      value: "",
    });
    expect(withField(draft, "last_played").operator).toBe(OPERATORS.date[0]);
    expect(withField(draft, "starred")).toMatchObject({
      operator: "is",
      value: "true",
    });
  });

  it("round-trips a stored rule set through the form", () => {
    const rules = {
      match: "any" as const,
      conditions: [
        { field: "genre" as const, operator: "is" as const, value: "Rock" },
        {
          field: "year" as const,
          operator: "between" as const,
          value: [1990, 1999] as [number, number],
        },
        { field: "starred" as const, operator: "is" as const, value: true },
      ],
      sort: "year" as const,
      order: "desc" as const,
      limit: 50,
    };
    expect(rulesFromDraft(draftFromRules(rules))).toEqual(rules);
  });

  it("holds an unfinished form back", () => {
    expect(rulesFromDraft(blankRules())).toBeNull();
    const draft = blankRules();
    draft.conditions = [];
    draft.limit = "1001";
    expect(rulesFromDraft(draft)).toBeNull();
    draft.limit = "";
    expect(rulesFromDraft(draft)).toEqual({
      match: "all",
      conditions: [],
      sort: null,
      order: "asc",
      limit: null,
    });
  });
});
