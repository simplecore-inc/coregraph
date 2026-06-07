import { strict as assert } from 'node:assert';
import { describe, it } from 'mocha';
import {
  createDaemonHealthTracker,
  type DaemonState,
} from '../../src/util/daemonHealth';

describe('DaemonHealthTracker', () => {
  it('initial state is offline', () => {
    const t = createDaemonHealthTracker();
    assert.equal(t.state, 'offline');
    t.dispose();
  });

  it('setState transitions and fires onDidChangeState', () => {
    const t = createDaemonHealthTracker();
    const events: DaemonState[] = [];
    t.onDidChangeState((s) => events.push(s));

    t.setState('starting');
    t.setState('online');
    t.setState('offline');

    assert.deepEqual(events, ['starting', 'online', 'offline']);
    assert.equal(t.state, 'offline');
    t.dispose();
  });

  it('setState to the same state does not re-fire', () => {
    const t = createDaemonHealthTracker();
    const events: DaemonState[] = [];
    t.onDidChangeState((s) => events.push(s));

    t.setState('online');
    t.setState('online');
    t.setState('online');

    assert.deepEqual(events, ['online']);
    t.dispose();
  });

  it('multiple subscribers all receive events', () => {
    const t = createDaemonHealthTracker();
    const a: DaemonState[] = [];
    const b: DaemonState[] = [];
    t.onDidChangeState((s) => a.push(s));
    t.onDidChangeState((s) => b.push(s));

    t.setState('online');

    assert.deepEqual(a, ['online']);
    assert.deepEqual(b, ['online']);
    t.dispose();
  });

  it('disposable unsubscribes a listener', () => {
    const t = createDaemonHealthTracker();
    const events: DaemonState[] = [];
    const sub = t.onDidChangeState((s) => events.push(s));

    t.setState('starting');
    sub.dispose();
    t.setState('online');

    assert.deepEqual(events, ['starting']);
    t.dispose();
  });

  it('dispose drops all listeners', () => {
    const t = createDaemonHealthTracker();
    const events: DaemonState[] = [];
    t.onDidChangeState((s) => events.push(s));
    t.dispose();
    t.setState('online');
    assert.deepEqual(events, []);
  });

  it('listener unsubscribing during dispatch does not skip siblings', () => {
    const t = createDaemonHealthTracker();
    const received: string[] = [];
    const sub1 = t.onDidChangeState(() => {
      received.push('first');
      sub1.dispose();
    });
    t.onDidChangeState(() => received.push('second'));

    t.setState('online');

    assert.deepEqual(received, ['first', 'second']);
    t.dispose();
  });
});
