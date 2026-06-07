import { strict as assert } from "node:assert";
import { describe, it } from "mocha";
import {
  decideWarning,
  type WarnThresholds,
} from "../../src/providers/commitWarningFormat";
import type { DiffResult } from "../../src/ipc/types";

function diff(p: Partial<DiffResult> = {}): DiffResult {
  return {
    base_ref: "HEAD",
    changed_files: [],
    total_reachable: 0,
    total_confidence_weighted: 0,
    inconsistencies_introduced: [],
    new_orphans: [],
    git_operation_in_progress: false,
    ...p,
  };
}

const th = (p: Partial<WarnThresholds> = {}): WarnThresholds => ({
  enabled: true,
  impactThreshold: 20,
  inconsistencyCount: 1,
  ...p,
});

describe("commitWarningProvider – decideWarning", () => {
  it("returns null when feature disabled", () => {
    const d = diff({ total_confidence_weighted: 100 });
    assert.equal(decideWarning(d, th({ enabled: false })), null);
  });

  it("returns null when under all thresholds", () => {
    const d = diff({ total_confidence_weighted: 5 });
    assert.equal(decideWarning(d, th()), null);
  });

  it("fires on impact threshold alone", () => {
    const d = diff({ total_confidence_weighted: 42 });
    const msg = decideWarning(d, th());
    assert.match(msg!, /impact 42\.0 > 20/);
  });

  it("fires on inconsistency count alone", () => {
    const d = diff({
      inconsistencies_introduced: [
        {
          category: "EnumMismatch",
          shared_value: "X",
          a: { name: "a", file: "/a", line: 0 },
          b: { name: "b", file: "/b", line: 0 },
        },
      ],
    });
    const msg = decideWarning(d, th({ impactThreshold: 100 }));
    assert.match(msg!, /1 inconsistency/);
  });

  it("combines both reasons when both fire", () => {
    const d = diff({
      total_confidence_weighted: 25,
      inconsistencies_introduced: [
        {
          category: "ConfigKey",
          shared_value: "db.url",
          a: { name: "a", file: "/a", line: 0 },
          b: { name: "b", file: "/b", line: 0 },
        },
        {
          category: "ApiPath",
          shared_value: "/x",
          a: { name: "c", file: "/c", line: 0 },
          b: { name: "d", file: "/d", line: 0 },
        },
      ],
    });
    const msg = decideWarning(d, th());
    assert.match(msg!, /impact 25\.0 > 20/);
    assert.match(msg!, /2 inconsistency/);
  });
});
