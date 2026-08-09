<script lang="ts">
  // Honest placeholder — real Library screen lands per the phase note below.
</script>

<div class="screen-header">
  <h3>Library</h3>
</div>
<div class="screen-body">
  <div class="blueprint placeholder-card">
    <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
    <h6>Library</h6>
    <p class="text-muted">
      Will list Beat Saber and other installed games with per-row version/bottle/status, an
      expandable detail panel, and a Run button that opens the pre-launch gate.
    </p>
    <p class="text-muted phase-note">Basic Run lands in Phase 3 · full library (add/edit, persistence) in Phase 4.</p>
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
