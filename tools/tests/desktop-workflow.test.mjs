import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { desktopPlan, executePlan } from '../desktop.mjs';

test('packaging runs the same native and browser checks as local validation', () => {
  const checks = desktopPlan('check');
  const packaging = desktopPlan('package');
  assert.deepEqual(packaging.slice(0, -1), checks);
  assert.equal(packaging.at(-1).at(-1), 'build:desktop');
  assert.ok(checks.some((command) => command.includes('agent-access.e2e.ts')));
});

test('a failed check prevents installer creation', () => {
  const calls = [];
  assert.throws(
    () =>
      executePlan(desktopPlan('package'), (command) => {
        calls.push(command);
        return calls.length === 2 ? 1 : 0;
      }),
    /Stopped before packaging/u,
  );
  assert.equal(calls.length, 2);
  assert.ok(calls.every((command) => !command.includes('build:desktop')));
});

test('daily development uses the actual desktop origin without creating an installer', () => {
  const plan = desktopPlan('dev');
  assert.equal(plan.length, 1);
  assert.deepEqual(plan, desktopPlan('preview'));
  assert.throws(() => desktopPlan('unchecked-package'));
});

test('native preview runs without packaging or a browser fixture', () => {
  const plan = desktopPlan('preview');
  assert.equal(plan.length, 1);
  assert.equal(plan[0].at(-1), 'preview');
});
