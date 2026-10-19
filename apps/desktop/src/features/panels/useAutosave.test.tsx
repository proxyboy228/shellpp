import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AUTOSAVE_DELAY_MS, useAutosave, type AutosaveOptions } from './useAutosave';

interface Deferred {
  promise: Promise<boolean>;
  resolve: (value: boolean) => void;
}

function deferred(): Deferred {
  let resolve: (value: boolean) => void = () => undefined;
  const promise = new Promise<boolean>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function setup(initial: Partial<AutosaveOptions> = {}) {
  const pending: Deferred[] = [];
  const save = vi.fn(() => {
    const next = deferred();
    pending.push(next);
    return next.promise;
  });
  const props: AutosaveOptions = { changeKey: 0, dirty: false, canSave: true, isNew: false, save, ...initial };
  const hook = renderHook((options: AutosaveOptions) => useAutosave(options), { initialProps: props });
  const update = (patch: Partial<AutosaveOptions>) => {
    Object.assign(props, patch);
    hook.rerender({ ...props });
  };
  const finish = async (value: boolean) => {
    await act(async () => {
      pending.shift()?.resolve(value);
    });
  };
  return { hook, save, update, finish };
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('useAutosave', () => {
  it('keeps a new record as a draft until its required fields are valid', () => {
    const { hook, save, update } = setup({ isNew: true, canSave: false });
    expect(hook.result.current.state).toBe('draft');
    update({ changeKey: 1, dirty: true });
    act(() => {
      vi.advanceTimersByTime(AUTOSAVE_DELAY_MS * 2);
    });
    expect(save).not.toHaveBeenCalled();
    expect(hook.result.current.state).toBe('draft');
  });

  it('saves 500 ms after the last edit and goes pending, saving, saved', async () => {
    const { hook, save, update, finish } = setup();
    expect(hook.result.current.state).toBe('saved');
    update({ changeKey: 1, dirty: true });
    expect(hook.result.current.state).toBe('pending');
    act(() => {
      vi.advanceTimersByTime(300);
    });
    update({ changeKey: 2 });
    act(() => {
      vi.advanceTimersByTime(AUTOSAVE_DELAY_MS - 1);
    });
    expect(save).not.toHaveBeenCalled();
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(save).toHaveBeenCalledTimes(1);
    expect(hook.result.current.state).toBe('saving');
    await finish(true);
    update({ dirty: false });
    expect(hook.result.current.state).toBe('saved');
  });

  it('creates a new record once it becomes valid', async () => {
    const { hook, save, update, finish } = setup({ isNew: true, canSave: false });
    update({ changeKey: 1, dirty: true, canSave: true });
    expect(hook.result.current.state).toBe('pending');
    act(() => {
      vi.advanceTimersByTime(AUTOSAVE_DELAY_MS);
    });
    expect(save).toHaveBeenCalledTimes(1);
    await finish(true);
    update({ isNew: false, dirty: false });
    expect(hook.result.current.state).toBe('saved');
  });

  it('saves edits made during a save right after it', async () => {
    const { save, update, finish } = setup();
    update({ changeKey: 1, dirty: true });
    act(() => {
      vi.advanceTimersByTime(AUTOSAVE_DELAY_MS);
    });
    expect(save).toHaveBeenCalledTimes(1);
    update({ changeKey: 2 });
    act(() => {
      vi.advanceTimersByTime(AUTOSAVE_DELAY_MS);
    });
    // The second save waits for the first one.
    expect(save).toHaveBeenCalledTimes(1);
    await finish(true);
    expect(save).toHaveBeenCalledTimes(2);
  });

  it('shows an error after a failed save and retries only after the next edit', async () => {
    const { hook, save, update, finish } = setup();
    update({ changeKey: 1, dirty: true });
    act(() => {
      vi.advanceTimersByTime(AUTOSAVE_DELAY_MS);
    });
    await finish(false);
    expect(hook.result.current.state).toBe('error');
    act(() => {
      vi.advanceTimersByTime(AUTOSAVE_DELAY_MS * 4);
    });
    expect(save).toHaveBeenCalledTimes(1);
    update({ changeKey: 2 });
    expect(hook.result.current.state).toBe('pending');
    act(() => {
      vi.advanceTimersByTime(AUTOSAVE_DELAY_MS);
    });
    expect(save).toHaveBeenCalledTimes(2);
  });

  it('treats a rejected save as a failure', async () => {
    const save = vi.fn(() => Promise.reject(new Error('boom')));
    const hook = renderHook((options: AutosaveOptions) => useAutosave(options), {
      initialProps: { changeKey: 0, dirty: false, canSave: true, isNew: false, save },
    });
    hook.rerender({ changeKey: 1, dirty: true, canSave: true, isNew: false, save });
    await act(async () => {
      vi.advanceTimersByTime(AUTOSAVE_DELAY_MS);
    });
    expect(hook.result.current.state).toBe('error');
  });

  it('saves pending changes at once when the panel closes', () => {
    const { hook, save, update } = setup();
    update({ changeKey: 1, dirty: true });
    hook.unmount();
    expect(save).toHaveBeenCalledTimes(1);
  });

  it('discards a draft when the panel closes', () => {
    const { hook, save, update } = setup({ isNew: true, canSave: false });
    update({ changeKey: 1, dirty: true });
    hook.unmount();
    expect(save).not.toHaveBeenCalled();
  });

  it('flushes on request', () => {
    const { hook, save, update } = setup();
    update({ changeKey: 1, dirty: true });
    act(() => {
      hook.result.current.flush();
    });
    expect(save).toHaveBeenCalledTimes(1);
    act(() => {
      vi.advanceTimersByTime(AUTOSAVE_DELAY_MS);
    });
    expect(save).toHaveBeenCalledTimes(1);
  });
});
