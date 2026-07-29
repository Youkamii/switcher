// npm 전역 설치(postinstall)에서 OS에 맞는 빌드를 미리 받아둔다.
// 저장소 안에서 도는 개발용 npm install에서는 조용히 아무것도 하지 않는다.
import { ensureDist, platformAsset } from "./dist.mjs";

if (process.env.npm_config_global === "true" && platformAsset()) {
  try {
    await ensureDist();
    console.log("switcher 준비 완료 — `switcher` 명령으로 실행하세요.");
  } catch (e) {
    // 설치 자체는 살려둔다 — 첫 `switcher` 실행이 다시 받기를 시도한다
    console.warn(`빌드 내려받기 실패 (${e.message}) — 첫 실행 때 다시 시도합니다.`);
  }
}
