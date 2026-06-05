# Doom-style assets (Freedoom)

The sprites and flats (`*.png`) in this directory are extracted from
**Freedoom** (Phase 1, v0.13.0) and re-encoded as PNG via the project's
PLAYPAL palette. They drive the raycaster in `src/doom.tsx`.

Freedoom is free/libre game content distributed under a **modified BSD
license**. Attribution: © the Freedoom project — https://freedoom.github.io/

- Sprites: imp (`TROO*`), demon (`SARG*`), shotgun + flash (`SHTG*`/`SHTF*`),
  pistol (`PISG*`/`PISF*`), soulsphere (`SOUL*`), barrel (`BAR1*`),
  candelabra (`CBRA*`), columns (`COL*`/`COLU*`/`ELEC*`), projectile/puff/etc.
- Flats (`flat_*.png`): 64×64 wall/floor/ceiling textures.
- `manifest.json`: per-asset file + dimensions + Doom sprite offsets.

No original (commercial) Doom assets are used.
