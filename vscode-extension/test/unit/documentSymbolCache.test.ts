import { strict as assert } from 'node:assert';
import { describe, it } from 'mocha';
import {
  createDocumentSymbolCache,
  type SymbolExecutor,
} from '../../src/util/documentSymbolCache';

// Minimal fakes: only the shape the cache touches. The cache never
// constructs vscode values itself (thanks to plain-object normalization),
// so structural typing is enough.

interface FakeUri {
  toString(): string;
}

interface FakeDoc {
  uri: FakeUri;
  version: number;
}

function makeDoc(uri: string, version: number): FakeDoc {
  return {
    uri: { toString: () => uri },
    version,
  };
}

function makeRange(line: number, col: number) {
  return {
    start: { line, character: col },
    end: { line, character: col },
  };
}

describe('DocumentSymbolCache', () => {
  it('calls executor once per (uri, version) key', async () => {
    let calls = 0;
    const exec: SymbolExecutor = async () => {
      calls++;
      return [];
    };
    const cache = createDocumentSymbolCache(exec as SymbolExecutor, 1000);

    const doc = makeDoc('file:///a.ts', 1) as unknown as import('vscode').TextDocument;
    await cache.get(doc);
    await cache.get(doc);
    await cache.get(doc);

    assert.equal(calls, 1);
    cache.dispose();
  });

  it('returns the SAME promise for concurrent calls on a hit', async () => {
    let resolveFn!: () => void;
    const gate = new Promise<void>((r) => {
      resolveFn = r;
    });
    let calls = 0;
    const exec: SymbolExecutor = async () => {
      calls++;
      await gate;
      return [];
    };
    const cache = createDocumentSymbolCache(exec as SymbolExecutor, 1000);
    const doc = makeDoc('file:///a.ts', 1) as unknown as import('vscode').TextDocument;

    const p1 = cache.get(doc);
    const p2 = cache.get(doc);
    assert.equal(p1, p2, 'concurrent calls should share the in-flight promise');

    resolveFn();
    await Promise.all([p1, p2]);
    assert.equal(calls, 1);
    cache.dispose();
  });

  it('refetches on version change', async () => {
    let calls = 0;
    const exec: SymbolExecutor = async () => {
      calls++;
      return [];
    };
    const cache = createDocumentSymbolCache(exec as SymbolExecutor, 1000);

    await cache.get(makeDoc('file:///a.ts', 1) as unknown as import('vscode').TextDocument);
    await cache.get(makeDoc('file:///a.ts', 2) as unknown as import('vscode').TextDocument);

    assert.equal(calls, 2);
    cache.dispose();
  });

  it('refetches after TTL expires', async () => {
    let calls = 0;
    const exec: SymbolExecutor = async () => {
      calls++;
      return [];
    };
    const cache = createDocumentSymbolCache(exec as SymbolExecutor, 20);
    const doc = makeDoc('file:///a.ts', 1) as unknown as import('vscode').TextDocument;

    await cache.get(doc);
    await new Promise((r) => setTimeout(r, 40));
    await cache.get(doc);

    assert.equal(calls, 2);
    cache.dispose();
  });

  it('normalizes SymbolInformation → DocumentSymbol shape', async () => {
    // SymbolInformation is flat: { name, kind, location: { range } }.
    // Our normalize() hands back plain objects shaped like DocumentSymbol:
    // { name, detail, kind, tags, range, selectionRange, children }.
    const range = makeRange(3, 0);
    const siInput = [
      {
        name: 'foo',
        kind: 11,
        location: { range, uri: { toString: () => 'file:///a.ts' } },
      },
    ];
    const exec: SymbolExecutor = async () => siInput as unknown as import('vscode').SymbolInformation[];
    const cache = createDocumentSymbolCache(exec as SymbolExecutor, 1000);
    const doc = makeDoc('file:///a.ts', 1) as unknown as import('vscode').TextDocument;

    const out = await cache.get(doc);
    assert.equal(out.length, 1);
    assert.equal(out[0].name, 'foo');
    assert.equal(out[0].kind, 11);
    assert.equal(out[0].range, range);
    assert.equal(out[0].selectionRange, range);
    assert.deepEqual(out[0].children, []);
    cache.dispose();
  });

  it('passes DocumentSymbol[] through unchanged (nested form)', async () => {
    const nested = [
      {
        name: 'Foo',
        detail: '',
        kind: 5,
        tags: [],
        range: makeRange(0, 0),
        selectionRange: makeRange(0, 0),
        children: [],
      },
    ];
    const exec: SymbolExecutor = async () => nested as unknown as import('vscode').DocumentSymbol[];
    const cache = createDocumentSymbolCache(exec as SymbolExecutor, 1000);
    const doc = makeDoc('file:///a.ts', 1) as unknown as import('vscode').TextDocument;

    const out = await cache.get(doc);
    assert.equal(out, nested as unknown, 'nested DocumentSymbol[] should pass through');
    cache.dispose();
  });

  it('caches empty arrays too (undefined executor result → [])', async () => {
    let calls = 0;
    const exec: SymbolExecutor = async () => {
      calls++;
      return undefined;
    };
    const cache = createDocumentSymbolCache(exec as SymbolExecutor, 1000);
    const doc = makeDoc('file:///a.ts', 1) as unknown as import('vscode').TextDocument;

    const r1 = await cache.get(doc);
    const r2 = await cache.get(doc);
    assert.deepEqual(r1, []);
    assert.deepEqual(r2, []);
    assert.equal(calls, 1);
    cache.dispose();
  });

  it('dispose clears entries', async () => {
    let calls = 0;
    const exec: SymbolExecutor = async () => {
      calls++;
      return [];
    };
    const cache = createDocumentSymbolCache(exec as SymbolExecutor, 1000);
    const doc = makeDoc('file:///a.ts', 1) as unknown as import('vscode').TextDocument;

    await cache.get(doc);
    cache.dispose();
    await cache.get(doc);
    assert.equal(calls, 2);
  });
});
