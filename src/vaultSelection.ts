export interface VaultProfile {
  provider: string;
  name: string;
  active: boolean;
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
