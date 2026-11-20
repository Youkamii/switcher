import test from "node:test";
import assert from "node:assert/strict";
import { readConsoleCredit } from "../src/consoleCredit.ts";

const org = "Demo’s Individual Org";
test("imports only the dashboard organization credits row", () => {
  const page = [org, "좋은 오후입니다", "조직 크레딧", "", "US$67.42", "이번 달 지출", "US$9.81", "/ 티어 한도 US$500"].join("\n");
  assert.equal(readConsoleCredit(page, "Demo's Individual Org"), 6742);
  assert.equal(readConsoleCredit(page, "Different"), undefined);
  assert.equal(readConsoleCredit(page + "\n" + page, org), undefined);
  assert.equal(readConsoleCredit(page.replace("조직 크레딧", "이번 달 지출"), org), undefined);
});
test("handles localized USD, thousands, a real zero, and unsupported amounts", () => {
  const parse = (line: string) => readConsoleCredit([org, "Organization credits", line].join("\r\n"), org);
  assert.equal(parse("US$0.00"), 0);
  assert.equal(parse("USD 2,345.6"), 234560);
  assert.equal(parse("-US$1.03"), -103);
  for (const invalid of ["€5.00", "CA$3.00", "10%", "US$0.999", "US$20만", "42", "US$9,00", "US$987654321012345.67"]) {
    assert.equal(parse(invalid), undefined);
  }
});
