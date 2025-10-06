const { test, describe } = require('node:test');
const assert = require('node:assert/strict');
const { normaliseCountdownTarget } = require('./time');

describe('normaliseCountdownTarget', () => {
  const base = new Date('2025-10-05T10:00:00Z');

  test('returns ISO string unchanged when already ISO', () => {
    const iso = '2025-10-05T12:34:56.000Z';
    assert.equal(normaliseCountdownTarget(iso, base), iso);
  });

  test('converts HH:MM into next occurrence today', () => {
    const result = normaliseCountdownTarget('18:30', base);
    assert.ok(result, 'should return an ISO string');
    const parsed = new Date(result);
    assert.equal(parsed.getHours(), 18);
    assert.equal(parsed.getMinutes(), 30);
    assert.ok(parsed > base, 'target should be in the future');
  });

  test('wraps to next day when time already passed', () => {
    const late = new Date('2025-10-05T23:15:00Z');
    const result = normaliseCountdownTarget('06:00', late);
    assert.ok(result);
    const parsed = new Date(result);
    assert.equal(parsed.getHours(), 6);
    assert.equal(parsed.getMinutes(), 0);
    assert.ok(parsed > late, 'should roll to the following day');
  });

  test('handles plain minute offsets', () => {
    const result = normaliseCountdownTarget('15', base);
    assert.ok(result);
    const parsed = new Date(result);
    const diffMs = parsed - base;
    assert.ok(diffMs >= 15 * 60 * 1000 && diffMs < 16 * 60 * 1000);
  });

  test('rejects invalid input', () => {
    assert.equal(normaliseCountdownTarget(''), null);
    assert.equal(normaliseCountdownTarget('abc'), null);
    assert.equal(normaliseCountdownTarget('-5'), null);
    assert.equal(normaliseCountdownTarget('27:00'), null);
    assert.equal(normaliseCountdownTarget(12345), null);
  });
});
