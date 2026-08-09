<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    active?: boolean;
    badge?: boolean;
    onclick: () => void;
    children: Snippet;
  }

  let { active = false, badge = false, onclick, children }: Props = $props();
</script>

<button type="button" class="nav-item" class:active onclick={onclick}>
  {@render children()}
  {#if badge}
    <span class="badge-dot"></span>
  {/if}
</button>

<style>
  .nav-item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 7px 10px;
    cursor: pointer;
    font: inherit;
    font-size: 13px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    font-family: var(--font-heading);
    font-weight: 600;
    color: var(--color-neutral-700);
    background: transparent;
    border: none;
    border-radius: 0;
    text-align: left;
  }
  .nav-item :global(svg) {
    flex: none;
  }
  .nav-item.active {
    background: color-mix(in srgb, var(--color-accent) 12%, transparent);
    color: var(--color-accent-700);
    box-shadow: inset 2px 0 0 var(--color-accent);
  }
  .nav-item:hover:not(.active) {
    background: color-mix(in srgb, var(--color-text) 5%, transparent);
  }
  .nav-item:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: -2px;
  }
  .badge-dot {
    margin-left: auto;
    width: 7px;
    height: 7px;
    flex: none;
    background: var(--color-accent-800);
  }
</style>
