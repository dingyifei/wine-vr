<script lang="ts">
  import type { Screen } from "../types";

  interface Props {
    onNavigate: (screen: Screen) => void;
  }

  let { onNavigate }: Props = $props();

  // Chip labels match the approved mockup (Sabrage.dc.html pipeChips) verbatim.
  const pipeChips: { n: string; arrow: boolean }[] = [
    { n: "Game (D3D11)", arrow: true },
    { n: "CrossOver / Wine", arrow: true },
    { n: "DXMT → Metal", arrow: true },
    { n: "oxrsys runtime", arrow: true },
    { n: "HEVC helper", arrow: true },
    { n: "ALVR → Quest 3", arrow: false },
  ];

  interface UpstreamCard {
    k: string; // kicker
    n: string; // name
    by: string;
    d: string; // description
    lic: string;
    src: string;
  }

  // Upstream authorship and licenses follow the approved mockup — this panel credits
  // the UPSTREAM projects; the dingyifei forks are noted in the descriptions.
  // (oxrsys's MPL-2.0 comes from the repo's own CLAUDE.md rules for ext/oxrsys.)
  const upstream: UpstreamCard[] = [
    {
      k: "Windows layer",
      n: "Wine / CrossOver",
      by: "Wine project · CodeWeavers",
      d: "Runs the Windows x64 game in a bottle.",
      lic: "LGPL-2.1",
      src: "winehq.org",
    },
    {
      k: "Graphics",
      n: "DXMT",
      by: "3Shain",
      d: "D3D11 → Metal translation, zero-copy. Deployed here as the sha256-pinned monofunc fork build.",
      lic: "MIT",
      src: "github.com/3Shain/dxmt",
    },
    {
      k: "OpenXR runtime",
      n: "OpenXR-OSX (oxrsys)",
      by: "Yannick Comte (demonixis)",
      d: "The OpenXR runtime for macOS. The dingyifei fork adds the embedded ALVR backend and the arm64 encoder helper.",
      lic: "MPL-2.0",
      src: "github.com/demonixis/OpenXR-OSX",
    },
    {
      k: "Wine bridge",
      n: "wineopenxr",
      by: "Valve",
      d: "Bridges OpenXR calls from Wine to the host. Forked here (via monofunc) for Metal fence sync + sRGB mapping.",
      lic: "LGPL-2.1",
      src: "github.com/ValveSoftware",
    },
    {
      k: "Streaming",
      n: "ALVR",
      by: "alvr-org community",
      d: "Streams frames + tracking to the Quest. v20.14.1 server core embedded in-process with reliability patches; stock client.",
      lic: "MIT",
      src: "github.com/alvr-org/ALVR",
    },
    {
      k: "Steam shim",
      n: "Goldberg emulator",
      by: "Mr_Goldberg",
      d: "steam_api64.dll drop-in — boots offline.",
      lic: "LGPL-3.0",
      src: "gitlab.com/Mr_Goldberg",
    },
  ];
</script>

<div class="about-screen">
  <h1 class="title">SABRAGE</h1>

  <div class="pipe-chips">
    {#each pipeChips as c (c.n)}
      <span class="chip-wrap">
        <span class="chip">{c.n}</span>
        {#if c.arrow}
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="var(--color-accent-700)" stroke-width="1.5">
            <line x1="5" y1="12" x2="19" y2="12"></line>
            <polyline points="12 5 19 12 12 19"></polyline>
          </svg>
        {/if}
      </span>
    {/each}
  </div>

  <h6 class="section-kicker">Built on — upstream projects</h6>
  <div class="upstream-grid">
    {#each upstream as u (u.n)}
      <div class="blueprint upstream-card">
        <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
        <div class="card-kicker card-kicker-sm">{u.k}</div>
        <div class="card-name">{u.n}</div>
        <div class="card-by">{u.by}</div>
        <div class="text-muted card-desc">{u.d}</div>
        <div class="card-footer">
          <span class="tag tag-outline">{u.lic}</span>
          <span class="text-muted card-src">{u.src}</span>
        </div>
      </div>
    {/each}
  </div>

  <div class="actions">
    <button class="btn btn-primary" onclick={() => onNavigate("library")}>
      <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor" stroke="none">
        <polygon points="6 3 20 12 6 21 6 3"></polygon>
      </svg>
      Open Library
    </button>
    <button class="btn btn-secondary" disabled title="Setup assistant lands in Phase 6">Setup</button>
  </div>
</div>

<style>
  .about-screen {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 44px 52px 32px;
  }
  .title {
    margin: 2px 0 6px;
    letter-spacing: 0.04em;
  }
  .pipe-chips {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 18px 0 30px;
    flex-wrap: wrap;
    font-family: var(--font-heading);
    font-weight: 600;
    font-size: 13px;
    letter-spacing: 0.04em;
  }
  .chip-wrap {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .chip {
    border: 1px solid var(--color-divider);
    padding: 4px 12px;
    text-transform: uppercase;
  }
  .section-kicker {
    color: var(--color-accent-700);
    margin-bottom: 12px;
  }
  .upstream-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 22px 26px;
  }
  .upstream-card {
    display: flex;
    flex-direction: column;
    padding: 14px 16px;
  }
  .card-kicker-sm {
    font-size: 9.5px;
  }
  .card-name {
    font-family: var(--font-heading);
    font-weight: 600;
    font-size: 20px;
    line-height: 1.15;
  }
  .card-by {
    font-size: 13px;
    font-weight: 500;
    color: var(--color-accent-700);
    margin: 3px 0 1px;
  }
  .card-desc {
    font-size: 12px;
    margin-bottom: 8px;
  }
  .card-footer {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: auto;
  }
  .card-src {
    font-size: 10.5px;
    font-family: ui-monospace, Menlo, monospace;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-top: 30px;
  }
</style>
