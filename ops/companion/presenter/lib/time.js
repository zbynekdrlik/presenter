function normaliseCountdownTarget(value, now = new Date()) {
  if (typeof value !== 'string') {
    return null;
  }

  const trimmed = value.trim();
  if (trimmed.length === 0) {
    return null;
  }

  if (trimmed.includes('T')) {
    const parsed = new Date(trimmed);
    if (!Number.isNaN(parsed.valueOf())) {
      return parsed.toISOString();
    }
  }

  const hhmmMatch = trimmed.match(/^(\d{1,2}):(\d{2})$/);
  if (hhmmMatch) {
    const hours = Number.parseInt(hhmmMatch[1], 10);
    const minutes = Number.parseInt(hhmmMatch[2], 10);
    if (!Number.isFinite(hours) || !Number.isFinite(minutes)) {
      return null;
    }
    if (hours < 0 || hours > 23 || minutes < 0 || minutes > 59) {
      return null;
    }

    const candidate = new Date(now.getTime());
    candidate.setSeconds(0, 0);
    candidate.setHours(hours, minutes, 0, 0);
    if (candidate <= now) {
      candidate.setDate(candidate.getDate() + 1);
    }
    return candidate.toISOString();
  }

  if (/^\d+$/.test(trimmed)) {
    const minutes = Number.parseInt(trimmed, 10);
    if (minutes > 0) {
      const future = new Date(now.getTime() + minutes * 60 * 1000);
      return future.toISOString();
    }
  }

  return null;
}

module.exports = {
  normaliseCountdownTarget,
};
