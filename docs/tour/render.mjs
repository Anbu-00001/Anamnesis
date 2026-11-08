// Render docs/tour/tour.html to PNG frames, one per 1/fps second, in headless Chrome.
//
//   node docs/tour/render.mjs <tour.html> <data.json> <out-dir> [fps]
//
// No dependencies: Chrome's DevTools protocol over Node's built-in WebSocket (Node 22+).
// The page draws the frame at time t when asked (render(t)), so frames never depend on
// how fast this machine is. It loads no network resource.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const [html, dataPath, outDir, fpsArg = "30"] = process.argv.slice(2);
if (!html || !dataPath || !outDir) {
  console.error("usage: node render.mjs <tour.html> <data.json> <out-dir> [fps]");
  process.exit(2);
}
const fps = Number(fpsArg);
const data = JSON.parse(fs.readFileSync(dataPath, "utf8"));
fs.mkdirSync(outDir, { recursive: true });

const profile = fs.mkdtempSync(path.join(os.tmpdir(), "tour-chrome-"));
const port = 9300 + Math.floor(Math.random() * 600);
const chrome = spawn(process.env.CHROME || "google-chrome", [
  "--headless=new", `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`,
  "--no-first-run", "--no-default-browser-check", "--hide-scrollbars", "--mute-audio",
  "--disable-background-networking", "--force-device-scale-factor=1",
  "--window-size=1280,720", "about:blank",
], { stdio: "ignore" });
const cleanup = () => { try { chrome.kill(); } catch {} fs.rmSync(profile, { recursive: true, force: true }); };
process.on("exit", cleanup);

async function target() {
  for (let i = 0; i < 100; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      const page = list.find(t => t.type === "page");
      if (page) return page.webSocketDebuggerUrl;
    } catch {}
    await new Promise(r => setTimeout(r, 100));
  }
  throw new Error("Chrome did not start");
}

const ws = new WebSocket(await target());
await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
let next = 0;
const pending = new Map();
const events = [];
ws.onmessage = m => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) {
    const { res, rej } = pending.get(msg.id);
    pending.delete(msg.id);
    msg.error ? rej(new Error(JSON.stringify(msg.error))) : res(msg.result);
  } else if (msg.method) events.push(msg.method);
};
const send = (method, params = {}) => new Promise((res, rej) => {
  const id = ++next;
  pending.set(id, { res, rej });
  ws.send(JSON.stringify({ id, method, params }));
});
const evaluate = async (expression) => {
  const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
  return r.result.value;
};

await send("Page.enable");
await send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 720, deviceScaleFactor: 1, mobile: false });
await send("Page.addScriptToEvaluateOnNewDocument", { source: "window.__capture = true;" });
await send("Page.navigate", { url: "file://" + path.resolve(html) });
for (let i = 0; i < 100 && !events.includes("Page.loadEventFired"); i++) await new Promise(r => setTimeout(r, 50));
await evaluate("document.fonts.ready.then(() => true)");
await evaluate(`setData(${JSON.stringify(data)}); true`);
const duration = await evaluate("DURATION");

const frames = Math.ceil(duration * fps);
for (let f = 0; f < frames; f++) {
  await evaluate(`render(${f / fps}); true`);
  const shot = await send("Page.captureScreenshot", { format: "png" });
  fs.writeFileSync(path.join(outDir, `f_${String(f).padStart(5, "0")}.png`), Buffer.from(shot.data, "base64"));
}
console.log(`${frames} frames at ${fps} fps (${duration}s)`);
ws.close();
cleanup();
process.exit(0);
