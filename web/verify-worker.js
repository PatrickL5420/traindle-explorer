// Web Worker: verifies a run off the main thread and streams progress. It downloads the
// run's solutions itself, so the page never has to hold them all.
importScripts("/vendor/math.js", "/verify-core.js");

self.onmessage = async (e) => {
  const { run, url } = e.data;
  try {
    const res = await fetch(url);
    if (!res.ok) throw new Error(`could not load ${url}`);
    const solutions = await res.json();
    const report = self.TraindleVerify.verifyRun(self.math, run, solutions, (done, total) =>
      self.postMessage({ type: "progress", done, total })
    );
    self.postMessage({ type: "done", report });
  } catch (err) {
    self.postMessage({ type: "error", message: String(err && err.message ? err.message : err) });
  }
};
