<script lang="ts">
  /** Background layer for the shell frame and ambient gradients. */
</script>

<div class="bg-effects">
  <div class="bg-orb bg-orb-top"></div>
  <div class="bg-orb bg-orb-right"></div>
</div>

<style>
  .bg-effects {
    height: 100%;
    left: 0;
    overflow: hidden;
    pointer-events: none;
    position: fixed;
    top: 0;
    width: 100%;
    z-index: 0;
  }

  .bg-orb {
    background: var(--accent-blue);
    border-radius: 50%;
    position: absolute;
  }

  .bg-orb-top {
    filter: blur(150px);
    height: var(--hy-shell-orb-top-size);
    left: 10%;
    opacity: 0.15;
    top: -10%;
    width: var(--hy-shell-orb-top-size);
  }

  .bg-orb-right {
    filter: blur(200px);
    height: var(--hy-shell-orb-right-size);
    opacity: 0.1;
    right: 5%;
    top: 40%;
    width: var(--hy-shell-orb-right-size);
  }
</style>
