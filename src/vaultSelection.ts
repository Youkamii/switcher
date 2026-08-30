export interface VaultProfile {
  provider: string;
  name: string;
  active: boolean;
  revision: number;
}

export interface VaultProfileChoice {
  profile: VaultProfile;
  selected: boolean;
  hideEmail: boolean;
}

export interface VaultSelection {
  provider: string;
  name: string;
  hideEmail: boolean;
}

export type VaultBoundary = "tab" | "close" | "hidden";

export interface VaultBoundaryPolicy {
  allowed: boolean;
  clearRecovery: boolean;
  clearImport: boolean;
}

export function vaultInteractionLocked(
  busy: boolean,
  recoveryPendingAck: boolean,
): boolean {
  return busy || recoveryPendingAck;
}

export function vaultBoundaryPolicy(
  busy: boolean,
  _boundary: VaultBoundary,
): VaultBoundaryPolicy {
  if (busy) {
    return { allowed: false, clearRecovery: false, clearImport: false };
  }

  return {
    allowed: true,
    clearRecovery: true,
    clearImport: true,
  };
}

export function selectedVaultProfiles(
  choices: readonly VaultProfileChoice[],
): VaultSelection[] {
  return choices
    .filter((choice) => choice.selected)
    .map(({ profile, hideEmail }) => ({
      provider: profile.provider,
      name: profile.name,
      hideEmail,
    }));
}

export function reconcileVaultProfileChoices(
  profiles: readonly VaultProfile[],
  previousChoices: readonly VaultProfileChoice[],
): VaultProfileChoice[] {
  const previousByProfile = new Map(
    previousChoices.map((choice) => [
      `${choice.profile.provider}\u0000${choice.profile.name}\u0000${choice.profile.revision}`,
      choice,
    ]),
  );

  return profiles.map((profile) => {
    const previous = previousByProfile.get(
      `${profile.provider}\u0000${profile.name}\u0000${profile.revision}`,
    );
    return {
      profile,
      selected: previous?.selected ?? false,
      hideEmail: previous?.hideEmail ?? true,
    };
  });
}
