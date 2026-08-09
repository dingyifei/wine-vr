<script lang="ts">
  // Honest placeholder — no logic yet; the real screen lands per the phase note below.
</script>

<div class="screen-header">
  <h3>Logs</h3>
</div>
<div class="screen-body">
  <div class="blueprint placeholder-card">
    <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
    <h6>Logs</h6>
    <p class="text-muted">
      Will tail the wine console, the oxrsys runtime log, and the ALVR session log for the
      running (or most recent) session, plus a filter over past runs in logs/.
    </p>
    <p class="text-muted phase-note">Lands in Phase 3, alongside Launch.</p>
  </div>
</div>

<style>
  .screen-header {
    padding: 22px 28px 14px;
    border-bottom: 1px solid var(--color-divider);
  }
  .screen-header h3 {
    margin: 0;
  }
  .screen-body {
    flex: 1;
    min-height: 0;
    display: grid;
    place-items: center;
    padding: 28px;
  }
  .placeholder-card {
    padding: 24px 28px;
    max-width: 420px;
    text-align: center;
  }
  .placeholder-card h6 {
    color: var(--color-accent-700);
    margin-bottom: 8px;
  }
  .placeholder-card p {
    font-size: 13px;
    margin: 0 0 8px;
  }
  .phase-note {
    font-size: 11.5px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    margin-bottom: 0 !important;
  }
</style>
