import { bench, describe } from "vitest";

import { benchClient, unwrapBenchResult } from "./_shared.js";

describe("stream", (): void => {
  bench("stream.streamEvents", async (): Promise<void> => {
    unwrapBenchResult("stream.streamEvents", await benchClient.stream.streamEvents());
  });
});
