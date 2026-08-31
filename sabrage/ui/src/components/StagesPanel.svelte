<script lang="ts">
  import { onMount } from "svelte";
  import { blocksMutation, type Stage } from "../ipc";
  import BottleSelect from "./BottleSelect.svelte";
  import { shQuote } from "../lib/demo";
  import { bottlesStore } from "../stores/bottles.svelte";
  import { sessionStore } from "../stores/session.svelte";
  import { stageStore } from "../stores/stage.svelte";

  interface Props {
    open: boolean;
    onClose: () => void;
  }
  let { open, onClose }: Props = $props();

  interface Card {
    stage: Stage;
    title: string;
    description: string;
    needsBottle: boolean;
  }

  const cards: Card[] = [
    {
      stage: "setup",
      title: "Setup",
      description:
        "Submodules, sha256-pinned binaries (DXMT + Goldberg), and the write-once oxrsys-runtime.toml.",
      needsBottle: false,
    },
    {
      stage: "build",
      title: "Build",
      description: "oxrsys (x86_64 + ALVR), the native-arm64 encoder helper, and wineopenxr.",
      needsBottle: false,
    },
    {
      stage: "install",
      title: "Install",
      description:
        "The DXMT + wineopenxr overlays, the bottle's OpenXR manifest, and the host registration (one sudo prompt).",
      needsBottle: true,
    },
  ];

  const bottles = $derived(bottlesStore.bottles);
  const bottlesLoaded = $derived(bottlesStore.bottlesLoaded);
  let selectedBottle = $state("");
  let copiedStage = $state<Stage | null>(null);

  /** Every whole-stage fix here (Setup/Build/Install) is refused by the
   * backend while a session is live — `deny_stage_while_session_live` in
   * `stages::run_stage` refuses even a dry run, since it exists to protect
   * artifacts the live session has open, not to avoid a real mutation. Only
   * disabling the real Run button (and leaving Dry-run offered) would just
   * move the failure from "disabled, explained" to "clicked, then refused". */
  const sessionLive = $derived(blocksMutation(sessionStore.status.phase));

  onMount(async () => {
    const state = await bottlesStore.load();
    const loaded = state?.bottles ?? [];
    selectedBottle = loaded.includes("Steam") ? "Steam" : (loaded[0] ?? "");
  });

  function demoCommand(card: Card): string {
    // `--bottle` is shown whenever a bottle is selected, not only for the
    // card that *requires* one (`needsBottle` gates the Run button and the
    // picker's visibility, not whether the flag applies — every demo.sh
    // stage accepts `--bottle`; see `run()` below, which passes it the same
    // way). A required-but-unselected bottle still shows the `<name>`
    // placeholder so the command reads as a template.
    if (card.needsBottle) {
      return `./demo.sh ${card.stage} --bottle ${shQuote(selectedBottle || "<name>")}`;
    }
    return selectedBottle
      ? `./demo.sh ${card.stage} --bottle ${shQuote(selectedBottle)}`
      : `./demo.sh ${card.stage}`;
  }

  async function copyCommand(card: Card) {
    try {
      await navigator.clipboard.writeText(demoCommand(card));
      copiedStage = card.stage;
      setTimeout(() => {
        if (copiedStage === card.stage) copiedStage = null;
      }, 1500);
    } catch {
      // Clipboard access can be denied in some webview contexts; the command
      // is still visible to select and copy by hand.
    }
  }

  function run(card: Card, dryRun: boolean) {
    // Pass whatever bottle is selected regardless of `card.needsBottle` — the
    // shared selector belongs to the whole panel, not just Install's card, so
    // running `setup` here matches `./demo.sh setup --bottle <name>` (which
    // uses the bottle, when given one, for its own-Beat-Saber-presence check)
    // rather than always forcing the "no bottle" path.
    stageStore.openGate({
      stage: card.stage,
      bottle: selectedBottle || null,
      dryRun,
    });
  }
</script>

{#if open}
  <div class="stages-backdrop">
    <div class="dialog blueprint elev-lg stages-dialog">
      <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
      <div>
        <div class="card-kicker">Pipeline</div>
        <div class="dialog-title">Run a stage</div>
        <p class="text-muted stages-intro">
          Each card runs the equivalent demo.sh command shown under it. Dry-run writes nothing and
          ends with a <code>-- plan (dry run)</code> list of every copy, write and command it would
          have made — including which copies it would skip because the bytes already match.
        </p>
      </div>

      <div class="stages-list">
        {#each cards as card (card.stage)}
          <div class="stage-card">
            <div class="stage-card-title">{card.title}</div>
            <p class="text-muted stage-card-desc">{card.description}</p>
            {#if card.needsBottle}
              <div class="field stage-bottle-field">
                <label for={`stages-bottle-${card.stage}`}>Bottle</label>
                <BottleSelect
                  id={`stages-bottle-${card.stage}`}
                  {bottles}
                  {bottlesLoaded}
                  bind:value={selectedBottle}
                />
              </div>
            {/if}
            <div class="stage-card-cmd-row">
              <code class="stage-card-cmd">{demoCommand(card)}</code>
              <button class="btn btn-ghost stage-copy-btn" onclick={() => copyCommand(card)}>
                {copiedStage === card.stage ? "Copied" : "Copy"}
              </button>
            </div>
            {#if sessionLive}
              <p class="text-muted stage-live-note">
                A session is live — Setup/Build/Install are refused until it's stopped.
              </p>
            {/if}
            <div class="stage-card-actions">
              <button class="btn btn-secondary" disabled={sessionLive} onclick={() => run(card, true)}>
                Dry-run
              </button>
              <button
                class="btn btn-primary"
                disabled={sessionLive || (card.needsBottle && !selectedBottle)}
                onclick={() => run(card, false)}
              >
                Run
              </button>
            </div>
          </div>
        {/each}
      </div>

      <div class="dialog-actions">
        <button class="btn btn-secondary" onclick={onClose}>Close</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .stages-backdrop {
    position: fixed;
    inset: 0;
    z-index: 39;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--color-neutral-900) 45%, transparent);
  }
  .stages-dialog {
    width: 640px;
    max-width: calc(100vw - 32px);
    max-height: 88vh;
    overflow: auto;
    background: var(--color-bg);
  }
  .stages-intro {
    font-size: 12.5px;
    margin: 6px 0 0;
  }
  .stages-list {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .stage-card {
    border: 1px solid var(--color-divider);
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .stage-card-title {
    font-family: var(--font-heading);
    font-weight: 600;
    font-size: 16px;
  }
  .stage-card-desc {
    font-size: 12.5px;
    margin: 0;
  }
  .stage-bottle-field {
    max-width: 220px;
  }
  .stage-bottle-field :global(select) {
    min-height: 30px;
    padding: 3px 8px;
    font-size: 13px;
  }
  .stage-card-cmd-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .stage-card-cmd {
    flex: 1;
    min-width: 0;
    font-family: ui-monospace, Menlo, monospace;
    font-size: 11.5px;
    background: var(--color-surface);
    border: 1px solid var(--color-divider);
    padding: 5px 8px;
    overflow-x: auto;
    white-space: pre;
  }
  .stage-copy-btn {
    flex: none;
    font-size: 11.5px;
    padding: 2px 8px;
  }
  .stage-live-note {
    font-size: 11.5px;
    margin: 0;
  }
  .stage-card-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
