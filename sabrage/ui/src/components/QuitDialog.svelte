<script lang="ts">
  // Three-way app-quit gate for a live session: stop and quit, keep running
  // and quit, or cancel. Mounted once at the app root (App.svelte); shown
  // when sessionStore.quitRequested (Rust side intercepts ExitRequested/
  // CloseRequested to prevent a live session dying with the process). Buttons
  // answer via sessionStore.resolveQuit; if this dialog sits unanswered for
  // 20 s, the *next* quit request is let through and takes the keep-running
  // answer (src-tauri commands.rs, QUIT_DIALOG_TIMEOUT). Exit policy:
  // sabrage/docs/design/critique.md.
  import { sessionStore } from "../stores/session.svelte";

  let stoppingAndQuitting = $state(false);

  async function chooseStop() {
    stoppingAndQuitting = true;
    try {
      await sessionStore.resolveQuit("stop");
    } finally {
      stoppingAndQuitting = false;
    }
  }

  function chooseKeep() {
    void sessionStore.resolveQuit("keep");
  }

  function chooseCancel() {
    void sessionStore.resolveQuit("cancel");
  }
</script>

{#if sessionStore.quitRequested}
  <div class="quit-backdrop">
    <div class="dialog blueprint elev-lg quit-dialog">
      <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
      <div class="card-kicker">Session</div>
      <div class="dialog-title">A Beat Saber session is running</div>
      <p class="dialog-body">
        Quitting Sabrage doesn't have to end the stream. Choose what happens to the running
        session before the app closes.
      </p>

      <div class="quit-options">
        <div class="quit-option">
          <div class="quit-option-title">Stop session and quit</div>
          <p class="text-muted quit-option-desc">
            Ends wine and the streamer cleanly, restores the Mac's audio output, and closes the
            ALVR dashboard — the same teardown as pressing Stop.
          </p>
        </div>
        <div class="quit-option">
          <div class="quit-option-title">Keep session running and quit</div>
          <p class="text-muted quit-option-desc">
            The game keeps streaming to the headset with no Sabrage window open. Audio stays
            routed to BlackHole until the session is stopped — Sabrage restores it automatically
            the next time it launches.
          </p>
        </div>
      </div>

      <div class="dialog-actions quit-actions">
        <button class="btn btn-ghost" onclick={chooseCancel} disabled={stoppingAndQuitting}>Cancel</button>
        <button class="btn btn-secondary" onclick={chooseKeep} disabled={stoppingAndQuitting}>
          Keep running and quit
        </button>
        <button class="btn btn-primary" onclick={chooseStop} disabled={stoppingAndQuitting}>
          {stoppingAndQuitting ? "Stopping…" : "Stop session and quit"}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .quit-backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--color-neutral-900) 50%, transparent);
  }
  .quit-dialog {
    width: 460px;
    max-width: calc(100vw - 32px);
    background: var(--color-bg);
  }
  .quit-options {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin: 4px 0 2px;
  }
  .quit-option {
    border-left: 2px solid var(--color-divider);
    padding-left: 10px;
  }
  .quit-option-title {
    font-family: var(--font-heading);
    font-weight: 600;
    font-size: 13.5px;
  }
  .quit-option-desc {
    font-size: 12px;
    margin: 2px 0 0;
  }
  .quit-actions {
    flex-wrap: wrap;
  }
</style>
