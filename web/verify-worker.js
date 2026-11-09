// Web Worker: verifies a run off the main thread and streams progress.
importScripts("/vendor/math.js", "/verify-core.js");

self.onmessage = (e) => {
  const { run, solutions } = e.data;
  try {
    const report = self.TraindleVerify.verifyRun(self.math, run, solutions, (done, total) =>
      self.postMessage({ type: "progress", done, total })
    );
    self.postMessage({ type: "done", report });
  } catch (err) {
    self.postMessage({ type: "error", message: String(err && err.message ? err.message : err) });
  }
};
