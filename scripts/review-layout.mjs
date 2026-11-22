// Read-only layout audit using the repository's actual HTML/CSS and synthetic data.
// No Tauri startup, account files, network login, or extra npm dependencies.
// TypeScript is the existing project compiler; it compiles the production geometry helper.
// Exit 1 means a desired layout invariant is currently violated; exit 2 is a harness error.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { spawnSync } from "node:child_process";
import ts from "typescript";

const root = fileURLToPath(new URL("../", import.meta.url));
const candidates = [
  process.env.CHROME_BIN,
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
  process.env.PROGRAMFILES && path.join(process.env.PROGRAMFILES, "Google/Chrome/Application/chrome.exe"),
  "/usr/bin/chromium",
  "/usr/bin/chromium-browser",
  "/usr/bin/google-chrome",
].filter(Boolean);
const chrome = candidates.find((candidate) => fs.existsSync(candidate));
if (!chrome) {
  console.error("Chrome/Chromium not found. Set CHROME_BIN to its executable path.");
  process.exit(2);
}
const outputAt = process.argv.indexOf("--output");
if (outputAt >= 0 && !process.argv[outputAt + 1]) {
  console.error("--output requires a JSON file path");
  process.exit(2);
}

// This function is serialized into an isolated browser document.
function audit() {
  const checks = [];
  const record = (id, passed, measured, expected) => checks.push({ id, passed, measured, expected });
  const rect = (el) => el.getBoundingClientRect();
  const main = document.getElementById("app");
  const shell = document.querySelector(".shell");
  const size = (width) => {
    document.documentElement.style.width = `${width}px`;
    shell.style.width = `${width}px`;
  };

  // Exercise every AI visibility combination while SYSTEM is enabled.
  document.body.className = "edge";
  for (const [claude, codex] of [[false, false], [true, false], [false, true], [true, true]]) {
    const enabled = { claude, codex, system: true };
    const geometry = edgeGaugeLayout(claude, codex, true);
    document.documentElement.style.setProperty("--edge-gauge-height", `${geometry.height}px`);
    document.documentElement.style.setProperty("--edge-tab-height", `${geometry.tabHeight}px`);
    document.getElementById("edge-gauge").dataset.separatedSystem = String(geometry.separatedSystem);
    const measured = [...document.querySelectorAll(".edge-gauge-row")].map((row) => {
      row.hidden = !enabled[row.dataset.provider];
      row.replaceChildren();
      if (!row.hidden) {
        const bar = document.createElement("div");
        bar.className = "edge-gauge-bar";
        row.append(bar);
      }
      return {
        provider: row.dataset.provider,
        hidden: row.hidden,
        display: getComputedStyle(row).display,
        height: rect(row).height,
      };
    });
    record(`hidden-ai-${Number(claude)}${Number(codex)}`,
      measured.every((row) => row.hidden ? row.height === 0 : row.height > 0), measured,
      "Every hidden AI row has zero layout height.");
  }

  document.body.className = "";
  size(360);
  main.className = "";
  // Keep fixtures away from the browser's initial pointer location.
  main.style.paddingTop = "100px";
  main.innerHTML = '<section><div class="card" tabindex="0">Example A<div class="card-actions"><button>Switch</button><button>Delete</button></div></div><div class="card">Example B</div></section>';
  const cards = main.querySelectorAll(".card");
  const before = rect(cards[1]).top;
  // Same class applied by the native macOS hover poller.
  cards[0].classList.add("hit-hover");
  const after = rect(cards[1]).top;
  record("hover-does-not-move-next-card", Math.abs(after - before) <= 1,
    { before, after, displacement: after - before }, "Next card moves at most 1 CSS px.");

  // Record toolbar/dock bounds separately; do not infer overflow from source strings.
  main.replaceChildren();
  main.style.paddingTop = "";
  document.getElementById("clambtn").hidden = false;
  for (const [mode, width, classes] of [
    ["normal", 360, "dock-open"],
    ["minimal", 120, "minimal locked dock-open"],
  ]) {
    document.body.className = classes;
    size(width);
    const bounds = rect(shell);
    const measured = [...document.querySelectorAll(".titlebar button, .dock-tray button")]
      .filter((el) => rect(el).width > 0)
      .map((el) => ({ id: el.id, x: rect(el).left, right: rect(el).right, y: rect(el).top }));
    record(`buttons-fit-${mode}`, measured.every((el) => el.x >= bounds.left && el.right <= bounds.right),
      measured, "Visible buttons stay within the shell horizontally.");
  }
  const report = { checks, passed: checks.filter((c) => c.passed).length,
    failed: checks.filter((c) => !c.passed).length };
  const out = document.createElement("pre");
  out.id = "layout-audit-result";
  out.textContent = btoa(JSON.stringify(report));
  document.body.append(out);
}

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "switcher-layout-audit-"));
try {
  const css = ["src/theme.css", "src/styles.css"].map((file) => fs.readFileSync(path.join(root, file), "utf8")).join("\n");
  const helper = ts.transpileModule(
    fs.readFileSync(path.join(root, "src/edgeGaugeLayout.ts"), "utf8").replace("export function", "function"),
    { compilerOptions: { target: ts.ScriptTarget.ES2022 } },
  ).outputText;
  const html = fs.readFileSync(path.join(root, "index.html"), "utf8")
    .replace(/<link\b[^>]*>/g, "")
    .replace(/<script\b[^>]*>[\s\S]*?<\/script>/g, "")
    .replace("</head>", `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'"><style>${css}</style></head>`)
    .replace("</body>", `<script>${helper}\n(${audit.toString()})()</script></body>`);
  const fixture = path.join(tmp, "fixture.html");
  fs.writeFileSync(fixture, html);
  const run = spawnSync(chrome, ["--headless", "--disable-gpu", "--no-first-run",
    "--no-default-browser-check", "--disable-background-networking", "--window-size=800,900",
    `--user-data-dir=${path.join(tmp, "browser")}`, "--dump-dom", pathToFileURL(fixture).href],
  { encoding: "utf8", timeout: 30_000, maxBuffer: 4 * 1024 * 1024 });
  if (run.error || run.status !== 0) throw new Error(`Browser audit failed (${run.error?.message ?? run.status})`);
  const encoded = run.stdout.match(/<pre id="layout-audit-result">([A-Za-z0-9+/=]+)<\/pre>/)?.[1];
  if (!encoded) throw new Error("Browser did not return layout measurements");
  const report = JSON.parse(Buffer.from(encoded, "base64").toString("utf8"));
  report.browser = spawnSync(chrome, ["--version"], { encoding: "utf8", timeout: 5000 }).stdout?.trim();
  report.platform = `${process.platform}-${process.arch}`;
  report.scope = "Actual repository HTML/CSS; synthetic DOM states; Chromium only; no Tauri IPC or native window checks.";
  if (outputAt >= 0) fs.writeFileSync(path.resolve(process.argv[outputAt + 1]), JSON.stringify(report, null, 2) + "\n");
  for (const check of report.checks) console.log(`${check.passed ? "PASS" : "FAIL"} ${check.id}`);
  console.log(`${report.passed} passed; ${report.failed} failed`);
  process.exitCode = report.failed ? 1 : 0;
} catch (error) {
  console.error(error.message);
  process.exitCode = 2;
} finally {
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 });
}
