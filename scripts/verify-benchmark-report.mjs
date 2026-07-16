import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

const reportPath = resolve(
  import.meta.dirname,
  '..',
  'reports',
  'trial-runs',
  'runtime-benchmark-latest.json',
);

const report = JSON.parse(await readFile(reportPath, 'utf8'));
const failures = [];

if (report.passed !== true) failures.push('benchmark did not report a passing result');
if (!(report.metrics?.chartCachedMs < report.metrics?.chartColdMs)) {
  failures.push('cached chart hashing was not faster than the cold run');
}
if (!(report.metrics?.exportP95Ms < report.thresholds?.exportP95Ms)) {
  failures.push('export p95 exceeded its threshold');
}
if (!(report.metrics?.journalEventsPerSecond > report.thresholds?.journalEventsPerSecond)) {
  failures.push('journal throughput did not exceed its threshold');
}
if (report.metrics?.journalRecoveredEvents !== report.workload?.journalEvents) {
  failures.push('the benchmark did not recover every journal event');
}

if (failures.length > 0) {
  console.error(`Benchmark report failed verification:\n- ${failures.join('\n- ')}`);
  process.exit(1);
}

console.log(JSON.stringify({
  passed: true,
  report: reportPath,
  metrics: report.metrics,
}, null, 2));
