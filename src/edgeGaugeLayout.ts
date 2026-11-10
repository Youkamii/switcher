/** Content dimensions of the collapsed handle; independent of WebView visibility. */
export function edgeGaugeLayout(claude: boolean, codex: boolean, system: boolean) {
  const aiCount = Number(claude) + Number(codex);
  const rows = aiCount + Number(system);
  const separatedSystem = system && aiCount > 0;
  const height = rows * 19 + Math.max(0, rows - 1) * 6 + (separatedSystem ? 10 : 0);
  return { height, tabHeight: Math.max(40, height + 30), separatedSystem };
}
