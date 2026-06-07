import { strict as assert } from "node:assert";
import { describe, it } from "mocha";
import {
  buildDiagnostics,
  groupByFile,
  inconsistencyToDiagnostics,
  orphanToDiagnostic,
} from "../../src/providers/diagnosticsFormat";
import type {
  OrphansResponse,
  InconsistenciesResponse,
  OrphanReport,
  InconsistencyReport,
} from "../../src/ipc/types";

describe("diagnosticsFormat – orphanToDiagnostic", () => {
  it("renders an orphan as Information with kind in message", () => {
    const o: OrphanReport = {
      name: "legacy_helper",
      kind: "Function",
      file: "/proj/dead.rs",
      line: 12,
    };
    const d = orphanToDiagnostic(o);
    assert.equal(d.file, "/proj/dead.rs");
    assert.equal(d.line, 12);
    assert.equal(d.severity, "information");
    assert.equal(d.source, "coregraph");
    assert.equal(d.code, "orphan");
    assert.match(d.message, /'legacy_helper'/);
    assert.match(d.message, /Function/);
    assert.match(d.message, /possibly dead code/);
  });
});

describe("diagnosticsFormat – inconsistencyToDiagnostics", () => {
  it("emits one diagnostic per implicated node (a + b) for EnumMismatch", () => {
    const r: InconsistencyReport = {
      category: "EnumMismatch",
      shared_value: "ACTIVE",
      a: { name: "PaymentStatus", file: "/proj/p.rs", line: 5 },
      b: { name: "OrderStatus", file: "/proj/o.rs", line: 9 },
    };
    const out = inconsistencyToDiagnostics(r);
    assert.equal(out.length, 2);
    assert.equal(out[0].file, "/proj/p.rs");
    assert.equal(out[0].line, 5);
    assert.equal(out[0].severity, "warning");
    assert.equal(out[0].code, "inconsistency.EnumMismatch");
    assert.equal(out[1].file, "/proj/o.rs");
    assert.equal(out[1].line, 9);
    // Both diagnostics share the same message.
    assert.equal(out[0].message, out[1].message);
    assert.match(out[0].message, /ACTIVE/);
    assert.match(out[0].message, /PaymentStatus/);
    assert.match(out[0].message, /OrderStatus/);
  });

  it("handles ApiPath inconsistencies with file paths in message", () => {
    const r: InconsistencyReport = {
      category: "ApiPath",
      shared_value: "/api/v1/users",
      a: { name: "controller", file: "/proj/api.ts", line: 1 },
      b: { name: "controller", file: "/proj/api2.ts", line: 1 },
    };
    const out = inconsistencyToDiagnostics(r);
    assert.equal(out.length, 2);
    assert.match(out[0].message, /\/api\/v1\/users/);
    assert.match(out[0].message, /\/proj\/api\.ts/);
    assert.match(out[0].message, /\/proj\/api2\.ts/);
  });

  it("handles ConfigKey inconsistencies", () => {
    const r: InconsistencyReport = {
      category: "ConfigKey",
      shared_value: "database.url",
      a: { name: "config_a", file: "/proj/cfg.toml", line: 3 },
      b: { name: "config_b", file: "/proj/app.yaml", line: 7 },
    };
    const out = inconsistencyToDiagnostics(r);
    assert.equal(out[0].code, "inconsistency.ConfigKey");
    assert.match(out[0].message, /database\.url/);
  });

  it("falls back to a generic message for unknown categories", () => {
    const r: InconsistencyReport = {
      category: "NewKind" as InconsistencyReport["category"],
      shared_value: "xyz",
      a: { name: "A", file: "/a.rs", line: 0 },
      b: { name: "B", file: "/b.rs", line: 0 },
    };
    const out = inconsistencyToDiagnostics(r);
    assert.match(out[0].message, /Inconsistency \(NewKind\)/);
  });
});

describe("diagnosticsFormat – groupByFile", () => {
  it("buckets diagnostics by file path", () => {
    const o1: OrphanReport = { name: "a", kind: "Fn", file: "/x.rs", line: 0 };
    const o2: OrphanReport = { name: "b", kind: "Fn", file: "/x.rs", line: 5 };
    const o3: OrphanReport = { name: "c", kind: "Fn", file: "/y.rs", line: 0 };
    const grouped = groupByFile([o1, o2, o3].map(orphanToDiagnostic));
    assert.equal(grouped.size, 2);
    assert.equal(grouped.get("/x.rs")!.length, 2);
    assert.equal(grouped.get("/y.rs")!.length, 1);
  });

  it("returns an empty map for empty input", () => {
    assert.equal(groupByFile([]).size, 0);
  });
});

describe("diagnosticsFormat – buildDiagnostics", () => {
  it("merges orphans + inconsistencies into a flat list", () => {
    const orphans: OrphansResponse = {
      count: 1,
      orphans: [{ name: "dead", kind: "Function", file: "/d.rs", line: 1 }],
    };
    const inconsistencies: InconsistenciesResponse = {
      count: 1,
      reports: [
        {
          category: "EnumMismatch",
          shared_value: "X",
          a: { name: "Foo", file: "/a.rs", line: 1 },
          b: { name: "Bar", file: "/b.rs", line: 2 },
        },
      ],
    };
    const all = buildDiagnostics(orphans, inconsistencies);
    // 1 orphan + 2 (per-side) inconsistency diagnostics.
    assert.equal(all.length, 3);
    const codes = all.map((d) => d.code).sort();
    assert.deepEqual(codes, [
      "inconsistency.EnumMismatch",
      "inconsistency.EnumMismatch",
      "orphan",
    ]);
  });
});
