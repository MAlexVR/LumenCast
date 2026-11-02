import { describe, expect, it } from "vitest";
import { createLatestSender } from "./latest";

const flush = () => new Promise((done) => setTimeout(done, 0));

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => (resolve = done));
  return { promise, resolve };
}

describe("createLatestSender", () => {
  it("sends the first value at once and only the newest of the rest afterwards", async () => {
    const sent: number[] = [];
    const gates = [deferred(), deferred(), deferred()];
    const submit = createLatestSender<number>((value) => {
      sent.push(value);
      return gates[sent.length - 1].promise;
    }, () => {});

    submit(1);
    submit(2);
    submit(3);
    submit(4);
    expect(sent).toEqual([1]);

    gates[0].resolve();
    await flush();
    expect(sent).toEqual([1, 4]);

    gates[1].resolve();
    await flush();
    expect(sent).toEqual([1, 4]);
  });

  it("reports a failed send and keeps accepting new values", async () => {
    const errors: unknown[] = [];
    const sent: number[] = [];
    const submit = createLatestSender<number>(
      async (value) => {
        sent.push(value);
        if (value === 1) throw new Error("rejected");
      },
      (error) => errors.push(error),
    );
    submit(1);
    await new Promise((done) => setTimeout(done, 0));
    expect(errors).toHaveLength(1);
    submit(2);
    await new Promise((done) => setTimeout(done, 0));
    expect(sent).toEqual([1, 2]);
  });
});
