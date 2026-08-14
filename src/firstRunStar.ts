export const STAR_CTA_LABEL = "Star me on GitHub";

export type GithubStarPromptChoice = "star" | "dismissed";
export type FirstRunStartupState = "star-prompt" | "ready";

export function firstRunStartupState(
  choice: GithubStarPromptChoice | null,
): FirstRunStartupState {
  return choice === null ? "star-prompt" : "ready";
}

type FirstRunStarDependencies = {
  persist: (choice: GithubStarPromptChoice) => Promise<unknown>;
  revealApp: () => void;
  starRepository: () => Promise<unknown>;
  reportError: (error: unknown) => void;
};

/// 선택 저장 실패가 앱 시작을 막지 않게 하고, Star 요청은 본문을 연 뒤 실행한다.
export async function completeFirstRunStarChoice(
  choice: GithubStarPromptChoice,
  dependencies: FirstRunStarDependencies,
): Promise<void> {
  try {
    await dependencies.persist(choice);
  } catch (error) {
    dependencies.reportError(error);
  }

  dependencies.revealApp();
  if (choice === "star") {
    void dependencies.starRepository().catch(dependencies.reportError);
  }
}
