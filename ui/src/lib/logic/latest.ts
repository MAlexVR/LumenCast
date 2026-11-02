/**
 * Sends values one at a time. While a send is in flight only the newest value
 * is kept, so dragging a slider never queues a backlog of stale requests.
 */
export function createLatestSender<T>(
  send: (value: T) => Promise<void>,
  onError: (error: unknown) => void,
): (value: T) => void {
  let inFlight = false;
  let pending: { value: T } | null = null;

  const run = async (value: T): Promise<void> => {
    inFlight = true;
    try {
      await send(value);
    } catch (error) {
      onError(error);
    } finally {
      inFlight = false;
    }
    if (pending) {
      const next = pending.value;
      pending = null;
      void run(next);
    }
  };

  return (value) => {
    if (inFlight) pending = { value };
    else void run(value);
  };
}
