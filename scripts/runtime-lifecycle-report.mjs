export const IDLE_WORKING_SET_TARGET_BYTES = 30 * 1024 * 1024;

/**
 * Builds the machine-readable release gate from measured invariants. Keeping
 * this pure makes it impossible for the executable trial to report PASS while
 * one of its own booleans says the opposite.
 */
export function buildRuntimeLifecycleReport({
  generatedAt = new Date().toISOString(),
  readiness,
  protocolVersion,
  workingSetBytes,
  duplicateMutexRejected,
  explicitSessionShutdown,
  parentExitCleanup,
  visibleConsoleRequested,
}) {
  const workingSetMeasured = Number.isFinite(workingSetBytes) && workingSetBytes > 0;
  const idleWorkingSetTargetMet =
    workingSetMeasured && workingSetBytes <= IDLE_WORKING_SET_TARGET_BYTES;
  const invariants = {
    readinessReceived: readiness === 'runtime.ready',
    protocolCompatible: protocolVersion === 3,
    workingSetMeasured,
    idleWorkingSetTargetMet,
    duplicateMutexRejected: duplicateMutexRejected === true,
    explicitSessionShutdown: explicitSessionShutdown === true,
    parentExitCleanup: parentExitCleanup === true,
    noVisibleConsoleRequested: visibleConsoleRequested === false,
  };
  return {
    schemaVersion: 3,
    generatedAt,
    passed: Object.values(invariants).every(Boolean),
    readiness,
    protocolVersion,
    workingSetBytes,
    idleWorkingSetTargetBytes: IDLE_WORKING_SET_TARGET_BYTES,
    idleWorkingSetTargetMet,
    duplicateMutexRejected,
    explicitSessionShutdown,
    parentExitCleanup,
    visibleConsoleRequested,
    invariants,
  };
}
