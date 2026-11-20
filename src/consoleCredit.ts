// Only the numeric row immediately following the dashboard's credit heading can
// be a balance. Spend, tier limits and API-key statistics can appear nearby.
export function readConsoleCredit(page: string, organization: string): number | undefined {
  if (!organization.trim() || page.length > 100_000) return;
  const lines = page.split(/\r?\n/).map(line => line.trim()).filter(Boolean);
  const normalize = (value: string) => value.replace(/’/g, "'");
  const headings = lines.map((line, index) =>
    /^(조직 크레딧|organization credits?)$/i.test(line) ? index : -1).filter(index => index >= 0);
  if (headings.length !== 1) return;
  const index = headings[0];
  if (!lines.slice(0, index).some(line => normalize(line) === normalize(organization))) return;
  const amount = /^(-?)(?:US\$|USD\s*|\$)\s*(-?)((?:\d{1,3}(?:,\d{3})+|\d+))(?:\.(\d{1,2}))?$/.exec(lines[index + 1] ?? "");
  if (!amount || (amount[1] && amount[2])) return;
  const minor = BigInt(amount[3].replace(/,/g, "")) * 100n + BigInt((amount[4] ?? "").padEnd(2, "0"));
  if (minor > BigInt(Number.MAX_SAFE_INTEGER)) return;
  return Number(minor) * (amount[1] || amount[2] ? -1 : 1);
}
