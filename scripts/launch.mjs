#!/usr/bin/env node
// `switcher` 명령 — 받아둔 빌드를 띄운다 (없으면 먼저 받아온다).
// 이미 떠 있으면 앱의 단일 인스턴스 가드가 기존 창을 앞으로 가져온다.
import { spawn } from "node:child_process";
import { ensureDist } from "./dist.mjs";

const entry = await ensureDist();
if (process.platform === "darwin") {
  spawn("open", [entry], { stdio: "ignore" });
} else {
  spawn(entry, [], { detached: true, stdio: "ignore" }).unref();
}
