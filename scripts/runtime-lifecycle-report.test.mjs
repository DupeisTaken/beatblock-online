import assert from 'node:assert/strict';
import test from 'node:test';
import {
  buildRuntimeLifecycleReport,
  IDLE_WORKING_SET_TARGET_BYTES,
} from './runtime-lifecycle-report.mjs';

const healthy = {
  generatedAt: '2026-07-20T00:00:00.000Z',
  readiness: 'runtime.ready',
  protocolVersion: 3,
  workingSetBytes: IDLE_WORKING_SET_TARGET_BYTES,
  duplicateMutexRejected: true,
  explicitSessionShutdown: true,
  parentExitCleanup: true,
  visibleConsoleRequested: false,
};

test('runtime lifecycle passes only when every measured invariant passes', () => {
  const report = buildRuntimeLifecycleReport(healthy);
  assert.equal(report.passed, true);
  assert.equal(report.idleWorkingSetTargetMet, true);
  assert.ok(Object.values(report.invariants).every(Boolean));
});

test('an over-budget or missing working-set sample fails the release gate', () => {
  for (const workingSetBytes of [0, Number.NaN, IDLE_WORKING_SET_TARGET_BYTES + 1]) {
    const report = buildRuntimeLifecycleReport({ ...healthy, workingSetBytes });
    assert.equal(report.passed, false);
    assert.equal(report.idleWorkingSetTargetMet, false);
  }
});

test('each lifecycle failure is reflected in the aggregate result', () => {
  for (const override of [
    { readiness: 'runtime.error' },
    { protocolVersion: 2 },
    { duplicateMutexRejected: false },
    { explicitSessionShutdown: false },
    { parentExitCleanup: false },
    { visibleConsoleRequested: true },
  ]) {
    assert.equal(buildRuntimeLifecycleReport({ ...healthy, ...override }).passed, false);
  }
});
